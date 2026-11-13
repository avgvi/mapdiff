//! Runs the differ over the recent history of the pinned real repositories
//! and checks every change set against git's own numbers.
//!
//! Skipped (with a note) for repositories that have not been fetched with
//! `.github/scripts/fetch-repos.sh`.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use mapdiff_core::ChangeState;
use mapdiff_gitdiff::{Comparison, Options};

const COMMITS: usize = 50;

/// Added and removed lines, and the number of files, per `git diff --numstat`.
fn git_numbers(root: &Path, commit: &str) -> (u32, u32, u32) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        // Git's defaults, whatever this machine's own git config says.
        .args([
            "-c",
            "diff.algorithm=myers",
            "-c",
            "diff.indentHeuristic=true",
            "-c",
            "diff.context=3",
        ])
        .args(["diff", "--numstat", "-M", &format!("{commit}^1"), commit])
        .output()
        .expect("git runs");
    let (mut added, mut removed, mut files) = (0, 0, 0);
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split('\t');
        // Binary files show `-` for both counts.
        added += fields
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        removed += fields
            .next()
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        files += 1;
    }
    (added, removed, files)
}

#[test]
fn recent_history_matches_git() {
    let cache = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/repos");
    for name in ["ripgrep", "zustand", "express", "httpx"] {
        let root = cache.join(name);
        if !root.is_dir() {
            eprintln!("skipping {name}: run .github/scripts/fetch-repos.sh to fetch it");
            continue;
        }
        let repo = mapdiff_gitdiff::open(&root).expect("repository opens");
        let commits = repo.log("HEAD", COMMITS + 1).expect("history is readable");

        let (mut checked, mut scopes, mut moves, mut renames) = (0, 0, 0, 0);
        let mut slowest = Duration::ZERO;
        let started = Instant::now();
        for commit in &commits {
            // The oldest commit of a shallow clone has no parent to compare with.
            let Ok(comparison) = Comparison::commit(&repo, commit) else {
                continue;
            };
            let one = Instant::now();
            let changes = mapdiff_gitdiff::change_set(&repo, &comparison, Options::default())
                .unwrap_or_else(|err| panic!("{name} {commit}: {err}"));
            slowest = slowest.max(one.elapsed());

            let problems = changes.violations();
            assert!(
                problems.is_empty(),
                "{name} {commit}:\n{}",
                problems
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let (added, removed, files) = git_numbers(&root, commit);
            assert_eq!(
                (
                    changes.stats.added,
                    changes.stats.removed,
                    changes.stats.files
                ),
                (added, removed, files),
                "{name} {commit}: lines added, lines removed and files differ from git"
            );

            checked += 1;
            scopes += changes.stats.scopes;
            for change in &changes.changes {
                match change.state {
                    ChangeState::MovedIn => moves += 1,
                    ChangeState::Renamed if change.is_scope() => renames += 1,
                    _ => {}
                }
            }
        }
        eprintln!(
            "{name}: {checked} commits, {scopes} changed scopes, {renames} renames, {moves} moves, \
             {:?} in all, slowest commit {slowest:?}",
            started.elapsed()
        );
        assert!(
            checked >= 40,
            "{name}: only {checked} commits could be compared"
        );
    }
}
