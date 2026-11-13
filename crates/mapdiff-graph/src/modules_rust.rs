//! Rust: how files form modules and crates, and where a type's members are.

use std::collections::HashMap;
use std::path::Path;

use mapdiff_core::{Language, ScopeKind};

use crate::index::Index;

pub struct RustModules {
    /// `mod name;` lines to the file that holds the module.
    mod_files: HashMap<usize, usize>,
    /// A module file to the module (or file) that declares it.
    parent_module: HashMap<usize, usize>,
    /// Crate names, as written in paths, to the crate's root file.
    crates: HashMap<String, usize>,
    /// Type names to the type and every `impl` block for it.
    types: HashMap<String, Vec<usize>>,
}

impl RustModules {
    pub fn new(index: &Index<'_>, root: &Path) -> Self {
        let mut mod_files = HashMap::new();
        let mut parent_module = HashMap::new();
        let mut types: HashMap<String, Vec<usize>> = HashMap::new();

        for (position, scope) in index.scopes.iter().enumerate() {
            if scope.language != Language::Rust {
                continue;
            }
            match scope.kind {
                ScopeKind::Module if scope.body_range.is_none() => {
                    let Some(name) = scope.name.as_deref() else {
                        continue;
                    };
                    let Some(parent) = index.parent[position] else {
                        continue;
                    };
                    if let Some(file) = module_file_for(index, position, name) {
                        mod_files.insert(position, file);
                        parent_module.insert(file, parent);
                    }
                }
                ScopeKind::Class | ScopeKind::Enum | ScopeKind::Interface | ScopeKind::Impl => {
                    if let Some(name) = scope.name.as_deref() {
                        types
                            .entry(base_type_name(name).to_owned())
                            .or_default()
                            .push(position);
                    }
                }
                _ => {}
            }
        }

        let mut crates = HashMap::new();
        let mut manifests: Vec<&str> = index
            .paths
            .keys()
            .copied()
            .filter(|path| path.rsplit('/').next() == Some("Cargo.toml"))
            .collect();
        manifests.sort();
        for manifest in manifests {
            let Ok(text) = std::fs::read_to_string(root.join(manifest)) else {
                continue;
            };
            let Some(name) = package_name(&text) else {
                continue;
            };
            let directory = Index::directory_of(manifest);
            let root_file = ["src/lib.rs", "src/main.rs"]
                .iter()
                .filter_map(|file| Index::join(directory, file))
                .find_map(|path| index.paths.get(path.as_str()).copied());
            if let Some(root_file) = root_file {
                crates.insert(name.replace('-', "_"), root_file);
            }
        }

        Self {
            mod_files,
            parent_module,
            crates,
            types,
        }
    }

    /// The file behind a `mod name;` line.
    pub fn module_file(&self, declaration: usize) -> Option<usize> {
        self.mod_files.get(&declaration).copied()
    }

    /// Every `mod name;` line in a file, with the file it names.
    pub fn declarations_in(&self, index: &Index<'_>, file: usize) -> Vec<(usize, usize)> {
        let mut found: Vec<(usize, usize)> = self
            .mod_files
            .iter()
            .filter(|(declaration, _)| index.file_of[**declaration] == file)
            .map(|(declaration, target)| (*declaration, *target))
            .collect();
        found.sort();
        found
    }

    pub fn crate_named(&self, name: &str) -> Option<usize> {
        self.crates.get(name).copied()
    }

    /// The module a scope is in: the nearest inline `mod { }` around it, or
    /// its file.
    fn module_of(&self, index: &Index<'_>, scope: usize) -> usize {
        let mut current = scope;
        loop {
            let is_module = match index.kind(current) {
                ScopeKind::File | ScopeKind::Document => true,
                ScopeKind::Module => index.scopes[current].body_range.is_some(),
                _ => false,
            };
            match (is_module, index.parent[current]) {
                (true, _) | (false, None) => return current,
                (false, Some(parent)) => current = parent,
            }
        }
    }

    fn parent_of_module(&self, index: &Index<'_>, module: usize) -> Option<usize> {
        match index.kind(module) {
            ScopeKind::Module => index.parent[module].map(|parent| self.module_of(index, parent)),
            _ => self
                .parent_module
                .get(&module)
                .map(|declarer| self.module_of(index, *declarer)),
        }
    }

    /// `crate`, `self` and `super` at the start of a path.
    pub fn keyword(&self, index: &Index<'_>, from: usize, name: &str) -> Option<usize> {
        if index.scopes[index.file_of[from]].language != Language::Rust {
            return None;
        }
        let module = self.module_of(index, from);
        match name {
            "self" => Some(module),
            "super" => self.parent_of_module(index, module),
            "crate" => {
                let mut top = module;
                // A cycle of `mod` lines would be a broken crate; do not hang on it.
                for _ in 0..64 {
                    match self.parent_of_module(index, top) {
                        Some(parent) => top = parent,
                        None => break,
                    }
                }
                Some(index.file_of[top])
            }
            _ => None,
        }
    }

    /// `super` and `self` after the start of a path: `super::super::name`.
    pub fn keyword_member(&self, index: &Index<'_>, module: usize, name: &str) -> Option<usize> {
        match name {
            "self" => Some(module),
            "super" => self.parent_of_module(index, module),
            _ => None,
        }
    }

    /// A member of a type that lives in another block for the same type:
    /// a method in an `impl`, a field of the `struct`.
    pub fn type_members(&self, index: &Index<'_>, target: usize, name: &str) -> Vec<usize> {
        let scope = &index.scopes[target];
        if scope.language != Language::Rust {
            return Vec::new();
        }
        let Some(blocks) = scope
            .name
            .as_deref()
            .and_then(|type_name| self.types.get(base_type_name(type_name)))
        else {
            return Vec::new();
        };
        let file = index.file_of[target];
        // The same file first: two types may share a name across a workspace.
        blocks
            .iter()
            .filter(|block| index.file_of[**block] == file)
            .chain(blocks.iter().filter(|block| index.file_of[**block] != file))
            .flat_map(|block| index.members_named(*block, name))
            .collect()
    }

    /// The type an `impl` block is for, when it is defined in the
    /// repository. Other scopes are returned as they are.
    pub fn primary_type(&self, index: &Index<'_>, own: usize) -> usize {
        let scope = &index.scopes[own];
        if scope.language != Language::Rust || scope.kind != ScopeKind::Impl {
            return own;
        }
        let Some(blocks) = scope
            .name
            .as_deref()
            .and_then(|name| self.types.get(base_type_name(name)))
        else {
            return own;
        };
        let file = index.file_of[own];
        let is_definition = |block: &&usize| index.kind(**block) != ScopeKind::Impl;
        blocks
            .iter()
            .filter(is_definition)
            .find(|block| index.file_of[**block] == file)
            .or_else(|| blocks.iter().find(is_definition))
            .copied()
            .unwrap_or(own)
    }
}

/// Where `mod name;` in a given place keeps its module.
fn module_file_for(index: &Index<'_>, declaration: usize, name: &str) -> Option<usize> {
    let file_path = index.scopes[index.file_of[declaration]].path.file.as_str();
    let directory = Index::directory_of(file_path);
    let stem = file_path
        .rsplit('/')
        .next()
        .unwrap_or(file_path)
        .trim_end_matches(".rs");

    // Inline modules around the line add their names to the path.
    let mut inline = Vec::new();
    let mut current = index.parent[declaration];
    while let Some(scope) = current {
        if index.kind(scope) != ScopeKind::Module {
            break;
        }
        inline.push(index.scopes[scope].name.clone().unwrap_or_default());
        current = index.parent[scope];
    }
    inline.reverse();
    let relative = |base: &str| {
        let mut path = base.to_owned();
        for part in inline.iter().map(String::as_str).chain([name]) {
            path = Index::join(&path, part)?;
        }
        Some(path)
    };

    // `src/a.rs` keeps its modules in `src/a/`; `mod.rs`, `lib.rs` and
    // `main.rs` keep theirs beside themselves. Other crate roots, such as
    // `src/bin/tool.rs`, also keep theirs beside themselves, so that place
    // is tried second.
    let own_directory = Index::join(directory, stem);
    let bases: Vec<String> = if matches!(stem, "mod" | "lib" | "main") {
        vec![directory.to_owned()]
    } else {
        own_directory
            .into_iter()
            .chain([directory.to_owned()])
            .collect()
    };
    bases.iter().find_map(|base| {
        let path = relative(base)?;
        [format!("{path}.rs"), format!("{path}/mod.rs")]
            .iter()
            .find_map(|candidate| index.paths.get(candidate.as_str()).copied())
    })
}

/// `Vec<T>` -> `Vec`, `fmt::Display` -> `Display`, `&'a mut Foo` -> `Foo`.
fn base_type_name(name: &str) -> &str {
    let name = name.split('<').next().unwrap_or(name);
    let name = name.rsplit("::").next().unwrap_or(name);
    name.rsplit(|c: char| !c.is_alphanumeric() && c != '_')
        .find(|part| !part.is_empty())
        .unwrap_or(name)
}

/// The `name` under `[package]` in a `Cargo.toml`.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some(value) = line.strip_prefix("name")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            let value = value.trim().trim_matches('"');
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_lose_generics_paths_and_references() {
        assert_eq!(base_type_name("Vec<T>"), "Vec");
        assert_eq!(base_type_name("fmt::Display"), "Display");
        assert_eq!(base_type_name("Tagged<'a, T>"), "Tagged");
        assert_eq!(base_type_name("&'a mut Foo"), "Foo");
        assert_eq!(base_type_name("Circle"), "Circle");
    }

    #[test]
    fn package_name_is_read_from_the_package_table_only() {
        let manifest = "[workspace]\nmembers = []\n\n[package]\nname = \"my-crate\"\nversion = \"1\"\n\n[dependencies]\nname = \"x\"\n";
        assert_eq!(package_name(manifest).as_deref(), Some("my-crate"));
        assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
    }
}
