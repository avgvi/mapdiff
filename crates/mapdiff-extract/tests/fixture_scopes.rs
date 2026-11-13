//! Snapshot tests: the scope tree of each hand-made fixture.
//!
//! These pin how each language maps to scopes. When a change to an extractor
//! is intended, review the diff with `cargo insta review`.

use std::path::{Path, PathBuf};

use mapdiff_core::ScopeTree;

fn fixture(language: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(language)
}

fn tree(language: &str) -> ScopeTree {
    // With tests: the fixtures show how every construct is read, and test
    // code is one of them.
    let tree =
        mapdiff_extract::extract_repo_with_tests(&fixture(language)).expect("fixture is readable");
    let problems = tree.violations();
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    tree
}

#[test]
fn rust_fixture() {
    insta::assert_snapshot!(tree("rust").render_text());
}

#[test]
fn python_fixture() {
    insta::assert_snapshot!(tree("python").render_text());
}

#[test]
fn typescript_fixture() {
    insta::assert_snapshot!(tree("typescript").render_text());
}

#[test]
fn javascript_fixture() {
    insta::assert_snapshot!(tree("javascript").render_text());
}

/// One small file as full JSON, to pin the serialised shape of a scope.
#[test]
fn json_shape() {
    let path = fixture("python").join("pkg/_internal.py");
    let source = std::fs::read(path).expect("fixture is readable");
    let tree = ScopeTree::new(mapdiff_extract::extract_file("pkg/_internal.py", &source));
    insta::assert_json_snapshot!(tree);
}

#[test]
fn a_repository_is_read_without_its_tests() {
    // The Rust fixture has a `mod tests` inside a source file.
    let with = mapdiff_extract::extract_repo_with_tests(&fixture("rust")).unwrap();
    let extracted = mapdiff_extract::extract_repo_parts(&fixture("rust")).unwrap();
    let without = &extracted.tree;
    let is_test = |scope: &mapdiff_core::Scope| scope.has(mapdiff_core::Modifier::Test);
    assert!(with.scopes.iter().any(is_test));
    assert!(!without.scopes.iter().any(is_test));
    assert!(
        without.violations().is_empty(),
        "{:?}",
        without.violations()
    );
    // Nothing else is lost, and no scope still lists a child that is gone.
    assert_eq!(
        without.scopes.len(),
        with.scopes.iter().filter(|scope| !is_test(scope)).count()
    );
    let ids: std::collections::HashSet<&str> = without
        .scopes
        .iter()
        .map(|scope| scope.id.as_str())
        .collect();
    for scope in &without.scopes {
        for child in &scope.children {
            assert!(ids.contains(child.as_str()), "{} lists {child}", scope.id);
        }
    }
    // The file says where its test code was, for whoever reads it again.
    assert_eq!(
        extracted.left_out.len(),
        1,
        "{:?}",
        extracted.left_out.keys()
    );
}
