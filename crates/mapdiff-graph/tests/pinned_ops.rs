//! Graph operations on the pinned real repositories: what must hold for any
//! repository, and ripgrep's crate structure against its own manifests.
//!
//! Skipped (with a note) for repositories that have not been fetched with
//! `.github/scripts/fetch-repos.sh`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use mapdiff_core::{Wire, WireKind};
use mapdiff_graph::{Graph, Level, Precise, View};

fn pinned(name: &str) -> Option<(PathBuf, Graph)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cache/repos")
        .join(name);
    if !root.is_dir() {
        eprintln!("skipping {name}: run .github/scripts/fetch-repos.sh to fetch it");
        return None;
    }
    // Fast tier only, so the result does not depend on indexes a developer
    // happens to have made.
    let graph =
        mapdiff_graph::build_with(&root, Precise::Off, &|_| {}).expect("repository is readable");
    Some((root, graph))
}

/// True when the wires have no loop once those marked as cycles are left out.
fn flows_one_way(wires: &[Wire]) -> bool {
    let kept: Vec<(&str, &str)> = wires
        .iter()
        .filter(|wire| wire.kind != WireKind::Cycle)
        .map(|wire| (wire.from.as_str(), wire.to.as_str()))
        .collect();
    let mut incoming: HashMap<&str, usize> = HashMap::new();
    for (from, to) in &kept {
        incoming.entry(from).or_insert(0);
        *incoming.entry(to).or_insert(0) += 1;
    }
    // Remove nodes nothing points at until none is left, or a loop remains.
    let mut ready: Vec<&str> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut removed = 0;
    while let Some(node) = ready.pop() {
        removed += 1;
        for (from, to) in &kept {
            if *from == node {
                let count = incoming.get_mut(to).expect("every end was counted");
                *count -= 1;
                if *count == 0 {
                    ready.push(to);
                }
            }
        }
    }
    removed == incoming.len()
}

#[test]
fn every_level_of_every_repository_is_sound() {
    for name in ["ripgrep", "zustand", "express", "httpx"] {
        let Some((_, graph)) = pinned(name) else {
            continue;
        };
        let ids: HashSet<&str> = graph
            .tree
            .scopes
            .iter()
            .map(|scope| scope.id.as_str())
            .collect();
        let total = graph.references.references.len() as u32;

        for level in [
            Level::Architecture,
            Level::Package,
            Level::Module,
            Level::Code,
        ] {
            let lifted = graph.lift(&View::level(level));
            assert_eq!(
                lifted,
                graph.lift(&View::level(level)),
                "{name} {level:?}: two runs differ"
            );

            let mut loops = 0;
            for (container, inside) in &lifted.containers {
                assert!(
                    ids.contains(container.as_str()),
                    "{name}: unknown container {container}"
                );
                assert!(
                    flows_one_way(&inside.wires),
                    "{name} {level:?}: {container} still has a loop after cycle marking"
                );
                loops += inside
                    .wires
                    .iter()
                    .filter(|wire| wire.kind == WireKind::Cycle)
                    .count();
                for wire in &inside.wires {
                    assert_ne!(wire.from, wire.to, "{name}: a wire from a node to itself");
                    assert!(ids.contains(wire.from.as_str()) && ids.contains(wire.to.as_str()));
                    assert!(!wire.samples.is_empty() && wire.samples.len() <= 3);
                    assert_eq!(wire.kinds.values().sum::<u32>(), wire.count);
                }
                for entry in &inside.entries {
                    assert!(entry.incoming >= entry.from && entry.from >= 1, "{entry:?}");
                }
            }
            // No reference is counted twice, and none is invented.
            let lifted_total: u32 = lifted.wires.iter().map(|wire| wire.count).sum();
            assert!(lifted_total <= total, "{name} {level:?}");
            if level == Level::Code {
                // Only references between a function and the named
                // functions nested in it are merged away here.
                assert!(
                    lifted_total * 100 >= total * 98,
                    "{name}: {lifted_total} of {total} references left at the lowest level"
                );
            }
            eprintln!(
                "{name} {level:?}: {} wires for {lifted_total} references, {} containers, {loops} wires close a loop",
                lifted.wires.len(),
                lifted.containers.len()
            );
        }
    }
}

/// The package name in a `Cargo.toml`, and the text of the file.
fn manifest(path: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && line.starts_with("name") {
            let name = line.split('"').nth(1)?.to_owned();
            return Some((name, text));
        }
    }
    None
}

#[test]
fn ripgrep_wires_between_crates_match_its_manifests() {
    let Some((root, graph)) = pinned("ripgrep") else {
        return;
    };
    // Each crate directory, with the name others depend on it by and the
    // manifest that lists what it depends on. The `rg` binary lives in
    // `crates/core` but is declared by the manifest at the root.
    let mut crates: BTreeMap<String, (String, String)> = BTreeMap::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        let directory = entry.file_name().to_string_lossy().into_owned();
        let own = entry.path().join("Cargo.toml");
        let declared = if own.is_file() {
            own
        } else {
            root.join("Cargo.toml")
        };
        if let Some(found) = manifest(&declared) {
            crates.insert(format!("crates/{directory}"), found);
        }
    }
    assert!(crates.len() >= 9, "{:?}", crates.keys());

    let lifted = graph.lift(&View::scopes(crates.keys().cloned()));
    assert!(
        lifted.wires.len() >= 15,
        "only {} wires between crates",
        lifted.wires.len()
    );
    // What each crate's manifest lists, then everything it can reach through
    // those: `rg` lists the `grep` crate, which re-exports `grep-cli` as
    // `grep::cli`, so `rg` using `crates/cli` is as declared.
    let mut reachable: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
    for (directory, (_, manifest_text)) in &crates {
        let listed = crates
            .iter()
            .filter(|(other, (name, _))| {
                *other != directory
                    && manifest_text.lines().any(|line| {
                        line.strip_prefix(name.as_str())
                            .is_some_and(|rest| rest.trim_start().starts_with('='))
                    })
            })
            .map(|(other, _)| other.as_str())
            .collect();
        reachable.insert(directory, listed);
    }
    loop {
        let mut grew = false;
        for directory in crates.keys() {
            let through: Vec<&str> = reachable[directory.as_str()]
                .iter()
                .flat_map(|listed| reachable[listed].iter().copied())
                .collect();
            let own = reachable.get_mut(directory.as_str()).unwrap();
            for further in through {
                grew |= own.insert(further);
            }
        }
        if !grew {
            break;
        }
    }
    for wire in &lifted.wires {
        assert!(
            reachable[wire.from.as_str()].contains(wire.to.as_str()),
            "{} uses {} ({} references) but no manifest lets it",
            wire.from,
            wire.to,
            wire.count
        );
    }
    // The other way round: a dependency a manifest lists shows up as a wire,
    // unless it is only there to be re-exported or switched on by a feature.
    let wired: HashSet<(&str, &str)> = lifted
        .wires
        .iter()
        .map(|wire| (wire.from.as_str(), wire.to.as_str()))
        .collect();
    let missing: Vec<(&str, &str)> = crates
        .iter()
        .flat_map(|(directory, (_, manifest_text))| {
            crates
                .iter()
                .filter(move |(_, (name, _))| {
                    // Up to `[dev-dependencies]`: what only tests need is
                    // not wired, since tests are not read.
                    let needed = manifest_text
                        .split("[dev-dependencies]")
                        .next()
                        .unwrap_or("");
                    needed.lines().any(|line| {
                        line.strip_prefix(name.as_str())
                            .is_some_and(|rest| rest.trim_start().starts_with('='))
                            && !line.contains("optional")
                    })
                })
                .map(move |(other, _)| (directory.as_str(), other.as_str()))
        })
        .filter(|pair| !wired.contains(pair))
        .collect();
    eprintln!("listed in a manifest but not wired: {missing:?}");
    // `grep` has no code of its own: a use of `grep::cli::x` is a use of
    // `crates/cli`, and that is where its wire ends.
    assert!(
        missing.iter().all(|(_, to)| *to == "crates/grep"),
        "{missing:?}"
    );
    // Crates do not depend on each other in a loop, so no wire closes one.
    let between = &lifted.containers["crates"].wires;
    assert!(
        between.iter().all(|wire| wire.kind == WireKind::Direct),
        "{between:?}"
    );

    // Hiding a crate changes nothing between the others, except that a pair
    // only it connected is now joined by a bridge.
    let whole = graph.lift(&View::level(Level::Architecture));
    let without =
        graph.lift(&View::level(Level::Architecture).hiding([String::from("crates/printer")]));
    let before: BTreeMap<(&str, &str), u32> = whole.containers["crates"]
        .wires
        .iter()
        .map(|wire| ((wire.from.as_str(), wire.to.as_str()), wire.count))
        .collect();
    for wire in &without.containers["crates"].wires {
        let ends = (wire.from.as_str(), wire.to.as_str());
        assert!(!wire.from.ends_with("printer") && !wire.to.ends_with("printer"));
        match wire.kind {
            WireKind::Bridge => assert!(!before.contains_key(&ends), "{ends:?} had its own wire"),
            _ => assert_eq!(before.get(&ends), Some(&wire.count), "{ends:?}"),
        }
    }

    // Every crate that others use says where it is entered.
    let used: HashSet<&str> = lifted.wires.iter().map(|wire| wire.to.as_str()).collect();
    let code = graph.lift(&View::level(Level::Package));
    for directory in used {
        let entries = &code.containers[directory].entries;
        assert!(
            !entries.is_empty(),
            "{directory} is used but lists no entry point"
        );
    }
}
