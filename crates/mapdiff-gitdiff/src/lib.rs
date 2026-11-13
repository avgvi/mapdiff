//! Reads two versions of a repository from git and builds a change set: the
//! diff expressed as changed scopes.

mod build;
mod git;
mod matching;
mod text;

use std::fmt;
use std::path::Path;

use mapdiff_core::{ChangeSet, Revision};

pub use git::{CommitInfo, PullRequest, Repo, Side};
pub use text::{DiffLine, DiffTag, Segment, diff_lines};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Leave out changes that are only whitespace, as `git diff -w` does.
    pub ignore_whitespace: bool,
}

/// The two versions to compare. The base is always a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub base: Revision,
    pub head: Revision,
    pub(crate) base_commit: String,
    pub(crate) head_side: Side,
}

impl Comparison {
    /// Works out what to compare from what the user typed.
    ///
    /// - nothing: the last commit against the working tree
    /// - `REV`: that commit against the working tree
    /// - `A..B`: commit `A` against commit `B`
    /// - `A...B`: where `B` branched off from `A`, against `B`
    ///
    /// With `staged`, the index stands in for the working tree.
    pub fn parse(repo: &Repo, spec: Option<&str>, staged: bool) -> Result<Self, Error> {
        let local = || {
            if staged {
                (Side::Index, String::from("index"))
            } else {
                (Side::WorkTree, String::from("working tree"))
            }
        };
        let or_head = |name: &str| if name.is_empty() { "HEAD" } else { name }.to_owned();

        let (base_label, base_commit, head_side, head_label) = match spec {
            None => {
                let (side, label) = local();
                (String::from("HEAD"), repo.resolve("HEAD")?, side, label)
            }
            Some(spec) => {
                if let Some((a, b)) = spec.split_once("...") {
                    let (a, b) = (or_head(a), or_head(b));
                    let head = repo.resolve(&b)?;
                    let base = repo.merge_base(&repo.resolve(&a)?, &head)?;
                    (
                        format!("merge base of {a} and {b}"),
                        base,
                        Side::Commit(head),
                        b,
                    )
                } else if let Some((a, b)) = spec.split_once("..") {
                    let (a, b) = (or_head(a), or_head(b));
                    let head = repo.resolve(&b)?;
                    (a.clone(), repo.resolve(&a)?, Side::Commit(head), b)
                } else {
                    let (side, label) = local();
                    (spec.to_owned(), repo.resolve(spec)?, side, label)
                }
            }
        };
        Ok(Self::new(base_label, base_commit, head_side, head_label))
    }

    /// The base side, for reading files as they were.
    pub fn base_side(&self) -> Side {
        Side::Commit(self.base_commit.clone())
    }

    /// The head side, for reading files as they are now.
    pub fn head_side(&self) -> &Side {
        &self.head_side
    }

    /// True when neither side can change under us: both are commits.
    pub fn is_fixed(&self) -> bool {
        matches!(self.head_side, Side::Commit(_))
    }

    /// One commit against its first parent: what that commit changed.
    pub fn commit(repo: &Repo, revision: &str) -> Result<Self, Error> {
        let head = repo.resolve(revision)?;
        let base = repo
            .first_parent(&head)
            .ok_or_else(|| Error(format!("{revision:?} has no parent commit to compare with")))?;
        Ok(Self::new(
            format!("{revision}^"),
            base,
            Side::Commit(head),
            revision.to_owned(),
        ))
    }

    fn new(base_label: String, base_commit: String, head_side: Side, head_label: String) -> Self {
        let head_commit = match &head_side {
            Side::Commit(commit) => Some(commit.clone()),
            Side::Index | Side::WorkTree => None,
        };
        Self {
            base: Revision {
                label: base_label,
                commit: Some(base_commit.clone()),
            },
            head: Revision {
                label: head_label,
                commit: head_commit,
            },
            base_commit,
            head_side,
        }
    }
}

/// Builds the change set for a comparison.
pub fn change_set(
    repo: &Repo,
    comparison: &Comparison,
    options: Options,
) -> Result<ChangeSet, Error> {
    build::change_set(repo, comparison, options)
}

/// The old and new source text of one entry of a change set, for showing a
/// before and after. Either side is `None` when the scope does not exist
/// there.
pub fn scope_texts(
    repo: &Repo,
    comparison: &Comparison,
    change: &mapdiff_core::Change,
) -> (Option<String>, Option<String>) {
    let slice = |side: &Side, path: &Option<String>, range: &Option<mapdiff_core::Range>| {
        let (path, range) = (path.as_deref()?, range.as_ref()?);
        let bytes = repo.read(side, path);
        let text = bytes.get(range.start_byte as usize..range.end_byte as usize)?;
        Some(String::from_utf8_lossy(text).into_owned())
    };
    (
        slice(
            &Side::Commit(comparison.base_commit.clone()),
            &change.old_path,
            &change.old_range,
        ),
        slice(&comparison.head_side, &change.new_path, &change.new_range),
    )
}

/// Opens the repository that contains `path`.
pub fn open(path: &Path) -> Result<Repo, Error> {
    Repo::open(path)
}
