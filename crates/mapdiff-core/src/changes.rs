//! The change set: a diff between two versions of a repository, expressed
//! as changed scopes instead of changed lines.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::{Access, Range, SCHEMA_VERSION, ScopeKind};

/// What happened to a scope between the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeState {
    Added,
    Removed,
    /// Same place, same name; its text or something below it changed.
    Modified,
    /// Same parent, new name.
    Renamed,
    /// Arrived here from another parent or file. Listed at its new place.
    MovedIn,
    /// Left for another parent or file. Listed at its old place.
    MovedOut,
}

/// What part of a scope changed. A scope can have several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Signature,
    Body,
    /// Only comments changed.
    Comments,
    /// Only whitespace or layout changed.
    Formatting,
    /// The access level changed, for example private to public.
    Access,
}

/// Added and removed line counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineCount {
    pub added: u32,
    pub removed: u32,
}

impl LineCount {
    pub fn is_zero(&self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

impl std::ops::AddAssign for LineCount {
    fn add_assign(&mut self, other: Self) {
        self.added += other.added;
        self.removed += other.removed;
    }
}

/// One changed scope, or a directory or file that contains changed scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub state: ChangeState,
    pub kinds: Vec<ChangeKind>,
    pub kind: ScopeKind,
    pub lang_kind: String,
    /// The last ID segment, or the path for directories and files.
    pub label: String,
    /// The label on the old side, when it differs (a rename).
    pub old_label: Option<String>,
    pub name: Option<String>,
    pub access: Access,
    /// The scope's ID on the base side. `None` when it was added.
    pub old_id: Option<String>,
    /// The scope's ID on the head side. `None` when it was removed.
    pub new_id: Option<String>,
    /// For moves: the ID of the same scope on the other side.
    pub counterpart: Option<String>,
    /// The file the scope is in on the base side.
    pub old_path: Option<String>,
    /// The file the scope is in on the head side.
    pub new_path: Option<String>,
    /// Lines changed in this scope itself, not counting scopes inside it.
    pub own: LineCount,
    /// Lines changed in this scope and everything inside it. For the root
    /// this equals what `git diff --shortstat` reports.
    pub total: LineCount,
    /// For a moved scope: how many lines differ between its old and new
    /// text. A pure move has zero here while `total` counts every line.
    pub moved_delta: Option<LineCount>,
    pub old_range: Option<Range>,
    pub new_range: Option<Range>,
    /// Identifies this exact change: it is the same only while both the old
    /// and the new text of the scope stay the same. A review mark is stored
    /// against it, so editing the scope again clears the mark.
    pub fingerprint: String,
    /// Position of the parent in `ChangeSet::changes`. `None` for the root.
    pub parent: Option<u32>,
    /// Positions of the children in `ChangeSet::changes`.
    pub children: Vec<u32>,
}

impl Change {
    /// The ID to show: the head side's when there is one.
    pub fn id(&self) -> &str {
        self.new_id
            .as_deref()
            .or(self.old_id.as_deref())
            .unwrap_or("")
    }

    /// True for scopes inside files; false for directories and files.
    pub fn is_scope(&self) -> bool {
        !matches!(
            self.kind,
            ScopeKind::Directory | ScopeKind::File | ScopeKind::Document
        )
    }

    pub fn is_container_only(&self) -> bool {
        matches!(self.kind, ScopeKind::Directory)
            || (self.state == ChangeState::Modified
                && self.own.is_zero()
                && !self.children.is_empty())
    }
}

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// What the user asked for: `main`, `HEAD~3`, `working tree`, `index`.
    pub label: String,
    /// The commit it resolved to. `None` for the working tree and the index.
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeStats {
    pub files: u32,
    /// Scopes inside files that changed themselves, not counting ones that
    /// only contain a changed scope. A move counts once.
    pub scopes: u32,
    pub added: u32,
    pub removed: u32,
}

/// Every change between two revisions, in pre-order: a parent always comes
/// before its children. The first entry is the repository root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub schema_version: u32,
    pub base: Revision,
    pub head: Revision,
    pub ignore_whitespace: bool,
    pub stats: ChangeStats,
    pub changes: Vec<Change>,
}

impl ChangeSet {
    pub fn new(
        base: Revision,
        head: Revision,
        ignore_whitespace: bool,
        changes: Vec<Change>,
    ) -> Self {
        let mut stats = ChangeStats::default();
        for change in &changes {
            match change.kind {
                ScopeKind::Directory => {}
                ScopeKind::File | ScopeKind::Document => stats.files += 1,
                _ => {
                    let changed_itself =
                        !change.own.is_zero() || change.state != ChangeState::Modified;
                    // A class added with ten members is one changed scope.
                    let inside_same = change
                        .parent
                        .map(|parent| &changes[parent as usize])
                        .is_some_and(|parent| parent.is_scope() && parent.state == change.state)
                        && matches!(change.state, ChangeState::Added | ChangeState::Removed);
                    if changed_itself && !inside_same && change.state != ChangeState::MovedOut {
                        stats.scopes += 1;
                    }
                }
            }
        }
        if let Some(root) = changes.first() {
            stats.added = root.total.added;
            stats.removed = root.total.removed;
        }
        Self {
            schema_version: SCHEMA_VERSION,
            base,
            head,
            ignore_whitespace,
            stats,
            changes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stats.files == 0
    }

    /// An indented, one-line-per-change rendering for people and for
    /// snapshot tests. It contains no commit hashes, so it is stable.
    pub fn render_text(&self) -> String {
        self.render_text_where(&|_| true)
    }

    /// Like `render_text`, but lists only the changes `keep` accepts, plus
    /// the directories, files and scopes that contain them. Line counts stay
    /// those of the whole change set.
    pub fn render_text_where(&self, keep: &dyn Fn(&Change) -> bool) -> String {
        // A change is shown when it is kept itself or holds a kept change.
        let mut shown = vec![false; self.changes.len()];
        for position in (0..self.changes.len()).rev() {
            let change = &self.changes[position];
            let itself = !change.is_container_only() && keep(change);
            let below = change.children.iter().any(|child| shown[*child as usize]);
            shown[position] = itself || below;
        }

        let mut out = String::new();
        let plural = |count: u32, word: &str| {
            if count == 1 {
                format!("1 {word}")
            } else {
                format!("{count} {word}s")
            }
        };
        let _ = writeln!(
            out,
            "{} in {}, +{} -{}",
            plural(self.stats.scopes, "changed scope"),
            plural(self.stats.files, "file"),
            self.stats.added,
            self.stats.removed
        );
        if shown.first() == Some(&true) {
            self.render_change(0, 0, &shown, &mut out);
        }
        out
    }

    fn render_change(&self, position: usize, depth: usize, shown: &[bool], out: &mut String) {
        let change = &self.changes[position];
        let _ = write!(out, "{}", "  ".repeat(depth));
        match &change.old_label {
            Some(old) => {
                let _ = write!(out, "{old} -> {}", change.label);
            }
            None => out.push_str(&change.label),
        }
        if change.kind == ScopeKind::Directory {
            out.push('/');
        } else {
            let _ = write!(out, " {}", state_name(change.state));
        }
        match (change.state, &change.counterpart) {
            (ChangeState::MovedIn, Some(from)) => {
                let _ = write!(out, " from {from}");
            }
            (ChangeState::MovedOut, Some(to)) => {
                let _ = write!(out, " to {to}");
            }
            _ => {}
        }
        if !change.kinds.is_empty() {
            let kinds: Vec<&str> = change
                .kinds
                .iter()
                .map(|kind| change_kind_name(*kind))
                .collect();
            let _ = write!(out, " [{}]", kinds.join(", "));
        }
        if change.total.added > 0 {
            let _ = write!(out, " +{}", change.total.added);
        }
        if change.total.removed > 0 {
            let _ = write!(out, " -{}", change.total.removed);
        }
        if let Some(delta) = change.moved_delta {
            if delta.is_zero() {
                out.push_str(" (text unchanged)");
            } else {
                let _ = write!(out, " (text changed +{} -{})", delta.added, delta.removed);
            }
        }
        out.push('\n');
        for child in &change.children {
            // The members of a type that was added or removed whole are
            // implied by the line for the type.
            let member = &self.changes[*child as usize];
            let implied = change.is_scope()
                && matches!(change.state, ChangeState::Added | ChangeState::Removed)
                && matches!(member.kind, ScopeKind::Field | ScopeKind::Variant);
            if shown[*child as usize] && !implied {
                self.render_change(*child as usize, depth + 1, shown, out);
            }
        }
    }

    /// Checks the structural rules and returns one message per broken rule.
    pub fn violations(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (position, change) in self.changes.iter().enumerate() {
            let id = change.id();
            match change.parent {
                None if position != 0 => {
                    problems.push(format!("{id}: only the root may lack a parent"))
                }
                Some(parent) if parent as usize >= position => {
                    problems.push(format!("{id}: appears before its parent"));
                }
                Some(parent)
                    if !self.changes[parent as usize]
                        .children
                        .contains(&(position as u32)) =>
                {
                    problems.push(format!("{id}: missing from its parent's children"));
                }
                _ => {}
            }
            let mut sum = change.own;
            for child in &change.children {
                match self.changes.get(*child as usize) {
                    Some(child) => sum += child.total,
                    None => problems.push(format!("{id}: child {child} does not exist")),
                }
            }
            if sum != change.total {
                problems.push(format!(
                    "{id}: total is not its own lines plus its children's"
                ));
            }
            match change.state {
                ChangeState::Added | ChangeState::MovedIn if change.new_id.is_none() => {
                    problems.push(format!("{id}: has no ID on the head side"));
                }
                ChangeState::Removed | ChangeState::MovedOut if change.old_id.is_none() => {
                    problems.push(format!("{id}: has no ID on the base side"));
                }
                _ => {}
            }
        }
        problems
    }
}

pub fn state_name(state: ChangeState) -> &'static str {
    match state {
        ChangeState::Added => "added",
        ChangeState::Removed => "removed",
        ChangeState::Modified => "modified",
        ChangeState::Renamed => "renamed",
        ChangeState::MovedIn => "moved in",
        ChangeState::MovedOut => "moved out",
    }
}

pub fn change_kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Signature => "signature",
        ChangeKind::Body => "body",
        ChangeKind::Comments => "comments",
        ChangeKind::Formatting => "formatting",
        ChangeKind::Access => "access",
    }
}
