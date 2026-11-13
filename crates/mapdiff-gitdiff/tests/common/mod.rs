//! What the differ's tests add to the shared throwaway repository.

// Each test file uses its own part of this module.
#![allow(dead_code)]

use mapdiff_core::ChangeSet;
use mapdiff_gitdiff::{Comparison, Options, Repo};
pub use mapdiff_testkit::TempRepo;

pub trait Diffing {
    fn open(&self) -> Repo;

    /// The change set of the last commit, checked against the model's rules
    /// and against git's own totals.
    fn last_commit(&self) -> ChangeSet;
}

impl Diffing for TempRepo {
    fn open(&self) -> Repo {
        mapdiff_gitdiff::open(self.path()).expect("temp repo opens")
    }

    fn last_commit(&self) -> ChangeSet {
        let repo = self.open();
        let comparison = Comparison::commit(&repo, "HEAD").expect("HEAD has a parent");
        let changes = mapdiff_gitdiff::change_set(&repo, &comparison, Options::default())
            .expect("change set builds");
        let problems = changes.violations();
        assert!(problems.is_empty(), "\n{}", problems.join("\n"));

        let (added, removed) = self.git_totals(&["HEAD^", "HEAD"]);
        assert_eq!(
            (changes.stats.added, changes.stats.removed),
            (added, removed),
            "totals differ from git diff --numstat:\n{}",
            changes.render_text()
        );
        changes
    }
}
