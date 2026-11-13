//! Walks a repository and builds one scope tree for all of it: directories,
//! files, and the scopes inside each file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use mapdiff_core::{Access, Hashes, Language, Range, Scope, ScopeKind, ScopePath, ScopeTree};
use rayon::prelude::*;
use xxhash_rust::xxh3::Xxh3;

/// Directories that never hold reviewable source, whatever `.gitignore` says.
const SKIPPED_DIRECTORIES: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
];

/// A repository's scopes, and what was left out of them.
pub struct Extracted {
    pub tree: ScopeTree,
    /// For each file that has test code in it next to other code (a Rust
    /// `mod tests`), the byte ranges of that test code. Its scopes are not
    /// in the tree, and whoever reads the file again for names must skip
    /// these ranges too.
    pub left_out: HashMap<String, Vec<(u32, u32)>>,
}

/// Extracts every file under `root` and returns one tree. Paths in the tree
/// are relative to `root`, which itself has the ID `.`.
///
/// Files ignored by git are left out, and so are tests (see `tests_rule`):
/// a test file is not opened at all. The result does not depend on the
/// order the file system lists entries in, or on how many threads ran.
pub fn extract_repo(root: &Path) -> io::Result<ScopeTree> {
    extract_repo_parts(root).map(|extracted| extracted.tree)
}

/// `extract_repo`, with the ranges of the test code it left out of files.
pub fn extract_repo_parts(root: &Path) -> io::Result<Extracted> {
    extract_with(root, crate::tests_rule::keeping_tests())
}

/// Every file under `root`, tests included: for looking at how tests
/// themselves are read.
pub fn extract_repo_with_tests(root: &Path) -> io::Result<ScopeTree> {
    extract_with(root, true).map(|extracted| extracted.tree)
}

fn extract_with(root: &Path, keep_tests: bool) -> io::Result<Extracted> {
    let mut files = list_files(root)?;
    if !keep_tests {
        files.retain(|(relative, _)| !crate::is_test_path(relative));
    }

    type OneFile = (String, Vec<Scope>, Vec<(u32, u32)>);
    let mut extracted: Vec<OneFile> = files
        .par_iter()
        .map(|(relative, absolute)| {
            let source = std::fs::read(absolute)?;
            let scopes = crate::extract_file(relative, &source);
            let (scopes, ranges) = match keep_tests {
                true => (scopes, Vec::new()),
                false => without_tests(scopes),
            };
            Ok((relative.clone(), scopes, ranges))
        })
        .collect::<io::Result<_>>()?;
    extracted.sort_by(|a, b| a.0.cmp(&b.0));
    let left_out = extracted
        .iter()
        .filter(|(_, _, ranges)| !ranges.is_empty())
        .map(|(path, _, ranges)| (path.clone(), ranges.clone()))
        .collect();
    let extracted: Vec<(String, Vec<Scope>)> = extracted
        .into_iter()
        .map(|(path, scopes, _)| (path, scopes))
        .collect();

    let root_name = root
        .canonicalize()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| String::from("."));
    Ok(Extracted {
        tree: assemble_tree(root_name, extracted),
        left_out,
    })
}

/// Takes the test scopes out of one file's scopes (a `#[test]` function, a
/// `#[cfg(test)]` module, a Python `test_` function), and returns the byte
/// ranges they covered.
fn without_tests(scopes: Vec<Scope>) -> (Vec<Scope>, Vec<(u32, u32)>) {
    use mapdiff_core::Modifier;
    // The first scope is the file: it stays, whatever it is.
    let gone: HashSet<String> = scopes
        .iter()
        .skip(1)
        .filter(|scope| scope.has(Modifier::Test))
        .map(|scope| scope.id.clone())
        .collect();
    if gone.is_empty() {
        return (scopes, Vec::new());
    }
    // A test inside a test is covered by the range of the outer one.
    let ranges = scopes
        .iter()
        .filter(|scope| gone.contains(&scope.id))
        .filter(|scope| {
            !scope
                .parent
                .as_ref()
                .is_some_and(|parent| gone.contains(parent))
        })
        .map(|scope| (scope.range.start_byte, scope.range.end_byte))
        .collect();
    let kept = scopes
        .into_iter()
        .filter(|scope| !gone.contains(&scope.id))
        .map(|mut scope| {
            scope.children.retain(|child| !gone.contains(child));
            scope
        })
        .collect();
    (kept, ranges)
}

/// A number that changes when a file `extract_repo` would read is added,
/// removed, resized or written to. It reads no file contents, so it is
/// cheap enough to check before reusing a tree that was built earlier.
pub fn stamp(root: &Path) -> io::Result<u64> {
    let mut hasher = Xxh3::new();
    for (relative, absolute) in list_files(root)? {
        let metadata = std::fs::metadata(&absolute)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        hasher.update(relative.as_bytes());
        hasher.update(&[0]);
        hasher.update(&metadata.len().to_le_bytes());
        hasher.update(&modified.to_le_bytes());
    }
    Ok(hasher.digest())
}

/// Repository-relative paths (with `/`) and their locations on disk, sorted.
fn list_files(root: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !SKIPPED_DIRECTORIES.contains(&name.as_ref())
        })
        .build();
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if !relative.is_empty() {
            files.push((relative, entry.path().to_path_buf()));
        }
    }
    files.sort();
    Ok(files)
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<String, Directory>,
    files: BTreeMap<String, Vec<Scope>>,
}

fn assemble_tree(root_name: String, extracted: Vec<(String, Vec<Scope>)>) -> ScopeTree {
    let mut root = Directory::default();
    for (path, scopes) in extracted {
        let mut parts: Vec<&str> = path.split('/').collect();
        let file = parts.pop().unwrap_or_default().to_owned();
        let mut directory = &mut root;
        for part in parts {
            directory = directory.directories.entry(part.to_owned()).or_default();
        }
        directory.files.insert(file, scopes);
    }
    let mut scopes = Vec::new();
    emit_directory(root, String::from("."), root_name, None, &mut scopes);
    ScopeTree::new(scopes)
}

/// Appends a directory and everything under it in pre-order, and returns the
/// position of the directory's own scope.
fn emit_directory(
    directory: Directory,
    id: String,
    name: String,
    parent: Option<String>,
    out: &mut Vec<Scope>,
) -> usize {
    let position = out.len();
    out.push(Scope {
        id: id.clone(),
        path: ScopePath {
            file: id.clone(),
            segments: Vec::new(),
        },
        kind: ScopeKind::Directory,
        lang_kind: String::from("directory"),
        language: Language::None,
        name: Some(name),
        access: Access::Public,
        modifiers: Vec::new(),
        range: Range::default(),
        name_range: None,
        header_range: Range::default(),
        body_range: None,
        signature: None,
        hashes: Hashes::default(),
        parent,
        children: Vec::new(),
    });

    // Directories and files share one name-sorted order.
    enum Entry {
        Directory(Directory),
        File(Vec<Scope>),
    }
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    for (name, child) in directory.directories {
        entries.insert(name, Entry::Directory(child));
    }
    for (name, scopes) in directory.files {
        entries.insert(name, Entry::File(scopes));
    }

    let mut children = Vec::new();
    let mut hashers = [Xxh3::new(), Xxh3::new(), Xxh3::new()];
    for (name, entry) in entries {
        let child_position = match entry {
            Entry::Directory(child) => {
                let child_id = if id == "." {
                    name.clone()
                } else {
                    format!("{id}/{name}")
                };
                emit_directory(child, child_id, name.clone(), Some(id.clone()), out)
            }
            Entry::File(mut scopes) => {
                let child_position = out.len();
                if let Some(file) = scopes.first_mut() {
                    file.parent = Some(id.clone());
                }
                out.extend(scopes);
                child_position
            }
        };
        let Some(child) = out.get(child_position) else {
            continue;
        };
        children.push(child.id.clone());
        // A directory's hashes cover its children's names and hashes, so
        // they change when anything below changes.
        let parts = [&child.hashes.text, &child.hashes.tokens, &child.hashes.code];
        for (hasher, part) in hashers.iter_mut().zip(parts) {
            hasher.update(name.as_bytes());
            hasher.update(&[0x1f]);
            hasher.update(part.as_bytes());
            hasher.update(&[0x1e]);
        }
    }

    let [text, tokens, code] = hashers.map(|hasher| format!("{:016x}", hasher.digest()));
    let body = code.clone();
    let scope = &mut out[position];
    scope.children = children;
    scope.hashes = Hashes {
        text,
        tokens,
        code,
        signature: format!("{:016x}", Xxh3::new().digest()),
        body,
    };
    position
}
