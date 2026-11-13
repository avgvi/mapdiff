//! Builds the reference graph of the pinned real repositories and checks
//! what must hold for any repository.
//!
//! Skipped (with a note) for repositories that have not been fetched with
//! `.github/scripts/fetch-repos.sh`.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

#[test]
fn pinned_repositories_give_a_sound_graph() {
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/repos");
    for name in ["ripgrep", "zustand", "express", "httpx"] {
        let root = cache.join(name);
        if !root.is_dir() {
            eprintln!("skipping {name}: run .github/scripts/fetch-repos.sh to fetch it");
            continue;
        }
        let started = Instant::now();
        let graph = mapdiff_graph::build(&root).expect("repository is readable");
        let elapsed = started.elapsed();

        let ids: HashSet<&str> = graph
            .tree
            .scopes
            .iter()
            .map(|scope| scope.id.as_str())
            .collect();
        let references = &graph.references.references;
        for reference in references {
            assert!(
                ids.contains(reference.from.as_str()),
                "{name}: unknown scope {}",
                reference.from
            );
            assert!(
                ids.contains(reference.to.as_str()),
                "{name}: unknown scope {}",
                reference.to
            );
            assert_ne!(
                reference.from, reference.to,
                "{name}: a scope refers to itself"
            );
            // The referring name lies inside the scope that makes the reference.
            let from = graph.tree.get(&reference.from).expect("checked above");
            assert!(
                from.range.start_byte <= reference.range.start_byte
                    && reference.range.end_byte <= from.range.end_byte,
                "{name}: the name at {:?} is outside {}",
                reference.range,
                reference.from
            );
        }
        // Sorted and without repeats, so output is stable.
        assert!(
            references.windows(2).all(|pair| pair[0] < pair[1]),
            "{name}: not sorted"
        );

        let again = mapdiff_graph::build(&root).expect("repository is readable");
        assert_eq!(
            graph.references, again.references,
            "{name}: two runs differ"
        );

        let stats = &graph.references.stats;
        eprintln!(
            "{name}: {} references from {} files, {} names unresolved, {elapsed:?}",
            stats.references, stats.files, stats.unresolved
        );
        // Tests are not read; in a small library most references are theirs.
        assert!(
            stats.references > 300,
            "{name}: only {} references",
            stats.references
        );
    }
}
