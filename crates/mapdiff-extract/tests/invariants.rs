//! The rules every extracted tree must obey, checked on edited sources and
//! on the pinned real repositories.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use mapdiff_core::{Modifier, Scope, ScopeKind, ScopeTree};

fn extract(path: &str, source: &str) -> ScopeTree {
    let tree = ScopeTree::new(mapdiff_extract::extract_file(path, source.as_bytes()));
    let problems = tree.violations();
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    tree
}

fn ids(tree: &ScopeTree) -> BTreeSet<String> {
    tree.scopes.iter().map(|scope| scope.id.clone()).collect()
}

fn scope<'a>(tree: &'a ScopeTree, id: &str) -> &'a Scope {
    tree.get(id)
        .unwrap_or_else(|| panic!("no scope {id} in:\n{}", tree.render_text()))
}

const BEFORE: &str = "\
/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn double(value: i32) -> i32 {
    add(value, value)
}
";

#[test]
fn editing_a_body_keeps_every_id() {
    let after = BEFORE.replace("    a + b\n", "    let sum = a + b;\n    sum\n");
    let (before, after) = (extract("m.rs", BEFORE), extract("m.rs", &after));
    assert_eq!(ids(&before), ids(&after));

    let (old, new) = (scope(&before, "m.rs::add()"), scope(&after, "m.rs::add()"));
    assert_eq!(
        old.hashes.signature, new.hashes.signature,
        "signature is untouched"
    );
    assert_ne!(old.hashes.code, new.hashes.code, "body changed");

    let (old, new) = (
        scope(&before, "m.rs::double()"),
        scope(&after, "m.rs::double()"),
    );
    assert_eq!(
        old.hashes.text, new.hashes.text,
        "the other function is untouched"
    );
}

#[test]
fn reformatting_changes_only_the_text_hash() {
    let after = BEFORE.replace("    a + b\n", "    a   +   b\n");
    let (before, after) = (extract("m.rs", BEFORE), extract("m.rs", &after));
    let (old, new) = (scope(&before, "m.rs::add()"), scope(&after, "m.rs::add()"));
    assert_ne!(old.hashes.text, new.hashes.text);
    assert_eq!(old.hashes.tokens, new.hashes.tokens);
    assert_eq!(old.hashes.code, new.hashes.code);
    assert_eq!(old.hashes.signature, new.hashes.signature);
}

#[test]
fn editing_a_comment_keeps_the_code_hash() {
    let after = BEFORE.replace("/// Adds two numbers.", "/// Returns the sum.");
    let (before, after) = (extract("m.rs", BEFORE), extract("m.rs", &after));
    let (old, new) = (scope(&before, "m.rs::add()"), scope(&after, "m.rs::add()"));
    assert_ne!(old.hashes.tokens, new.hashes.tokens);
    assert_eq!(old.hashes.code, new.hashes.code);
    assert_eq!(old.hashes.signature, new.hashes.signature);
}

#[test]
fn changing_a_signature_changes_the_signature_hash() {
    let after = BEFORE.replace("pub fn add(a: i32, b: i32)", "pub fn add(a: i64, b: i32)");
    let (before, after) = (extract("m.rs", BEFORE), extract("m.rs", &after));
    assert_eq!(
        ids(&before),
        ids(&after),
        "a signature change is not a rename"
    );
    let (old, new) = (scope(&before, "m.rs::add()"), scope(&after, "m.rs::add()"));
    assert_ne!(old.hashes.signature, new.hashes.signature);
}

#[test]
fn a_change_inside_a_string_changes_the_code_hash() {
    let before = "export const greeting = `hello ${name}, welcome`;\n";
    let after = "export const greeting = `hello ${name}, goodbye`;\n";
    let (before, after) = (extract("m.ts", before), extract("m.ts", after));
    let (old, new) = (
        scope(&before, "m.ts::greeting"),
        scope(&after, "m.ts::greeting"),
    );
    assert_ne!(old.hashes.code, new.hashes.code);
}

#[test]
fn the_same_source_always_gives_the_same_tree() {
    assert_eq!(extract("m.rs", BEFORE), extract("m.rs", BEFORE));
}

#[test]
fn a_file_with_a_syntax_error_keeps_the_parts_that_parse() {
    let source = "\
def good():
    return 1


def broken(:
    return 2


def also_good():
    return 3
";
    let tree = extract("m.py", source);
    assert!(
        tree.get("m.py::good()").is_some(),
        "\n{}",
        tree.render_text()
    );
    assert!(
        tree.get("m.py::also_good()").is_some(),
        "\n{}",
        tree.render_text()
    );
    assert_eq!(tree.scopes[0].kind, ScopeKind::File);
    assert!(
        tree.scopes.iter().any(|scope| scope.has(Modifier::Error)),
        "the broken region is marked:\n{}",
        tree.render_text()
    );
}

#[test]
fn unparseable_and_binary_input_never_panics() {
    for (path, bytes) in [
        ("garbage.rs", &b"}}}} fn ((( {{{{ \" unterminated"[..]),
        ("garbage.py", &b"\t\t)))) def class :::"[..]),
        ("garbage.ts", &b"<<<<< >>>>> export export {{{"[..]),
        ("garbage.js", &b"function ( { [ `${"[..]),
        ("binary.rs", &[0xff, 0xfe, 0x00, 0x80, 0x81][..]),
        ("empty.py", &b""[..]),
        ("notes.txt", &b"plain text"[..]),
    ] {
        let tree = ScopeTree::new(mapdiff_extract::extract_file(path, bytes));
        let problems = tree.violations();
        assert!(problems.is_empty(), "{path}:\n{}", problems.join("\n"));
        assert!(
            !tree.scopes.is_empty(),
            "{path} has at least its file scope"
        );
    }
}

/// Runs over the pinned real repositories when they have been fetched with
/// `.github/scripts/fetch-repos.sh`. Skipped (with a note) when they are absent.
#[test]
fn pinned_repositories_extract_cleanly() {
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/repos");
    for name in ["ripgrep", "zustand", "express", "httpx"] {
        let root = cache.join(name);
        if !root.is_dir() {
            eprintln!("skipping {name}: run .github/scripts/fetch-repos.sh to fetch it");
            continue;
        }
        let started = Instant::now();
        let tree = mapdiff_extract::extract_repo(&root).expect("repository is readable");
        let elapsed = started.elapsed();

        let problems = tree.violations();
        assert!(
            problems.is_empty(),
            "{name}: {} problems, first ones:\n{}",
            problems.len(),
            problems
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_eq!(
            tree,
            mapdiff_extract::extract_repo(&root).unwrap(),
            "{name}: two runs differ"
        );

        let files = tree
            .scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::File)
            .count();
        let errors = tree
            .scopes
            .iter()
            .filter(|scope| scope.has(Modifier::Error))
            .count();
        eprintln!(
            "{name}: {} scopes in {files} source files, {errors} unparsed regions, {elapsed:?}",
            tree.scopes.len()
        );
        assert!(files > 10, "{name}: expected source files, found {files}");
    }
}
