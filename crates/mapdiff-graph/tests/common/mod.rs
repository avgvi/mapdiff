//! Builds a throwaway repository from literal files and lists its references.

// Each test file uses its own part of this module.
#![allow(dead_code)]

use std::collections::BTreeSet;

use mapdiff_core::RefKind;
use mapdiff_graph::Graph;
use mapdiff_testkit::TempRepo;

pub fn graph(name: &str, files: &[(&str, &str)]) -> Graph {
    let repo = TempRepo::new(name);
    for (path, content) in files {
        repo.write(path, content);
    }
    // The labelled lists describe the fast tier, so no index is read here.
    let graph = mapdiff_graph::build_with(repo.path(), mapdiff_graph::Precise::Off, &|_| {})
        .expect("graph builds");
    let problems = graph.tree.violations();
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));

    // Every reference must join two scopes that exist.
    for reference in &graph.references.references {
        assert!(
            graph.tree.get(&reference.from).is_some(),
            "unknown scope {}",
            reference.from
        );
        assert!(
            graph.tree.get(&reference.to).is_some(),
            "unknown scope {}",
            reference.to
        );
    }
    graph
}

fn kind_name(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Call => "call",
        RefKind::TypeUse => "type",
        RefKind::Import => "import",
        RefKind::Inherit => "inherit",
        RefKind::Implement => "implement",
        RefKind::Macro => "macro",
        RefKind::Access => "access",
    }
}

/// Every reference as `from -> to [kind]`, sorted and without repeats.
pub fn edges(graph: &Graph) -> BTreeSet<String> {
    graph
        .references
        .references
        .iter()
        .map(|reference| {
            format!(
                "{} -> {} [{}]",
                reference.from,
                reference.to,
                kind_name(reference.kind)
            )
        })
        .collect()
}

/// Checks the references against a hand-written list, reporting both what
/// is missing and what was not expected.
pub fn assert_edges(graph: &Graph, expected: &[&str]) {
    let found = edges(graph);
    let expected: BTreeSet<String> = expected.iter().map(|edge| (*edge).to_owned()).collect();
    let missing: Vec<&str> = expected.difference(&found).map(String::as_str).collect();
    let extra: Vec<&str> = found.difference(&expected).map(String::as_str).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "\nmissing:\n  {}\nnot expected:\n  {}\n",
        missing.join("\n  "),
        extra.join("\n  "),
    );
}
