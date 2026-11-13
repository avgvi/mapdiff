//! Builds the change set: reads both sides, matches scopes, gives every
//! changed line to one scope, and assembles the tree of changes.

use std::collections::BTreeMap;

use mapdiff_core::{
    Access, Change, ChangeKind, ChangeSet, ChangeState, LineCount, Scope, ScopeKind,
};
use rayon::prelude::*;

use crate::git::{FileDiff, FileStatus, Repo, Side as GitSide};
use crate::matching::{Matcher, Side, base_segment};
use crate::text::line_delta;
use crate::{Comparison, Error, Options};

pub fn change_set(
    repo: &Repo,
    comparison: &Comparison,
    options: Options,
) -> Result<ChangeSet, Error> {
    let base_side = GitSide::Commit(comparison.base_commit.clone());
    let files = repo.diff(
        &comparison.base_commit,
        &comparison.head_side,
        options.ignore_whitespace,
    )?;

    // Read and parse both sides of every changed file, in parallel.
    let parsed: Vec<(Option<Parsed>, Option<Parsed>)> = files
        .par_iter()
        .map(|file| {
            let read = |side: &GitSide, path: &Option<String>| {
                path.as_deref().map(|path| {
                    let source = repo.read(side, path);
                    let scopes = mapdiff_extract::extract_file(path, &source);
                    Parsed { scopes, source }
                })
            };
            (
                read(&base_side, &file.old_path),
                read(&comparison.head_side, &file.new_path),
            )
        })
        .collect();

    let (mut old, mut new) = (Side::default(), Side::default());
    let mut pairs = Vec::new();
    for (old_file, new_file) in parsed {
        let old_file = old_file.map(|parsed| old.add_file(parsed.scopes, parsed.source));
        let new_file = new_file.map(|parsed| new.add_file(parsed.scopes, parsed.source));
        pairs.push((old_file, new_file));
    }

    let mut matcher = Matcher::new(&old, &new);
    for (old_file, new_file) in &pairs {
        if let (Some(old_file), Some(new_file)) = (old_file, new_file) {
            matcher.pair(*old_file, *new_file);
        }
    }
    matcher.match_moves();

    // Give every changed line to the innermost scope that contains it.
    let mut removed = vec![0u32; old.scopes.len()];
    let mut added = vec![0u32; new.scopes.len()];
    for (file, (old_file, new_file)) in files.iter().zip(&pairs) {
        let old_lines = old_file.map(|scope| LineOwners::new(&old, scope));
        let new_lines = new_file.map(|scope| LineOwners::new(&new, scope));
        for hunk in &file.hunks {
            if let Some(owners) = &old_lines {
                for line in hunk.old_start..hunk.old_start + hunk.old_len {
                    removed[owners.owner(line)] += 1;
                }
            }
            if let Some(owners) = &new_lines {
                for line in hunk.new_start..hunk.new_start + hunk.new_len {
                    added[owners.owner(line)] += 1;
                }
            }
        }
    }

    let builder = Builder {
        matcher: &matcher,
        added: &added,
        removed: &removed,
        options,
    };
    let mut root = Directory::default();
    for (file, (old_file, new_file)) in files.iter().zip(&pairs) {
        let node = builder.file(file, *old_file, *new_file);
        let mut parts: Vec<&str> = file.path().split('/').collect();
        parts.pop();
        let mut directory = &mut root;
        for part in parts {
            directory = directory.directories.entry(part.to_owned()).or_default();
        }
        directory.files.push(node);
    }

    let mut changes = Vec::new();
    flatten(
        directory_node(root, String::from("."), String::from(".")),
        None,
        &mut changes,
    );
    Ok(ChangeSet::new(
        comparison.base.clone(),
        comparison.head.clone(),
        options.ignore_whitespace,
        changes,
    ))
}

struct Parsed {
    scopes: Vec<Scope>,
    source: Vec<u8>,
}

/// Finds the scope that owns a line of one file.
struct LineOwners<'a> {
    side: &'a Side,
    file: usize,
    /// For each line, the byte to look up: its first character that is not
    /// whitespace, or the line start when the line is blank.
    probes: Vec<u32>,
}

impl<'a> LineOwners<'a> {
    fn new(side: &'a Side, file: usize) -> Self {
        let source = &side.sources[side.source_of[file]];
        let mut probes = Vec::new();
        let mut line_start = 0usize;
        for line in source.split_inclusive(|byte| *byte == b'\n') {
            let indent = line
                .iter()
                .position(|byte| !byte.is_ascii_whitespace())
                .unwrap_or(0);
            probes.push((line_start + indent) as u32);
            line_start += line.len();
        }
        Self { side, file, probes }
    }

    /// The innermost scope containing the line. Lines past the end of the
    /// file belong to the file itself.
    fn owner(&self, line: u32) -> usize {
        let Some(&probe) = self.probes.get(line as usize) else {
            return self.file;
        };
        let mut scope = self.file;
        loop {
            let children = &self.side.children[scope];
            let after = children
                .partition_point(|child| self.side.scopes[*child].range.start_byte <= probe);
            let inside = after
                .checked_sub(1)
                .map(|position| children[position])
                .filter(|child| probe < self.side.scopes[*child].range.end_byte);
            match inside {
                Some(child) => scope = child,
                None => return scope,
            }
        }
    }
}

/// A change with its children still nested, before flattening.
struct Node {
    change: Change,
    children: Vec<Node>,
    /// Orders siblings: by position on the head side; removed scopes sit
    /// after the last surviving scope that came before them.
    order: (u32, u8, u32),
}

struct Builder<'a> {
    matcher: &'a Matcher<'a>,
    added: &'a [u32],
    removed: &'a [u32],
    options: Options,
}

impl Builder<'_> {
    fn old(&self, scope: usize) -> &Scope {
        &self.matcher.old.scopes[scope]
    }

    fn new_scope(&self, scope: usize) -> &Scope {
        &self.matcher.new.scopes[scope]
    }

    /// A file is always listed, even when nothing in it could be attributed
    /// (a mode change, a pure rename), so the file count matches git's.
    fn file(&self, diff: &FileDiff, old: Option<usize>, new: Option<usize>) -> Node {
        let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
        let directory = |path: &str| {
            path.rsplit_once('/')
                .map(|(directory, _)| directory.to_owned())
        };

        let mut node = match (old, new) {
            (Some(old), Some(new)) => self
                .pair(old, new)
                .unwrap_or_else(|| self.leaf(Some(old), Some(new))),
            (None, Some(new)) => self.one_sided(None, Some(new)),
            (Some(old), None) => self.one_sided(Some(old), None),
            (None, None) => unreachable!("git lists a file on at least one side"),
        };
        node.change.label = name(diff.path());
        node.change.old_label = None;
        // What changed is said on the scopes inside, not on the file.
        node.change.kinds.clear();
        node.change.state = match diff.status {
            FileStatus::Added => ChangeState::Added,
            FileStatus::Deleted => ChangeState::Removed,
            FileStatus::Modified => ChangeState::Modified,
            FileStatus::Renamed => ChangeState::Renamed,
        };
        if let (FileStatus::Renamed, Some(old_path), Some(new_path)) =
            (diff.status, &diff.old_path, &diff.new_path)
        {
            node.change.old_label = Some(if directory(old_path) == directory(new_path) {
                name(old_path)
            } else {
                old_path.clone()
            });
        }
        node
    }

    /// A scope that exists on both sides, in the same place. `None` when
    /// nothing about it changed.
    fn pair(&self, old: usize, new: usize) -> Option<Node> {
        let (a, b) = (self.old(old), self.new_scope(new));
        let mut node = self.leaf(Some(old), Some(new));
        node.children = self.children(Some(old), Some(new));

        // Imports and blocks have numbers, not names: a new number only
        // means something was added or removed above.
        let numbered = matches!(a.kind, ScopeKind::Import | ScopeKind::Block);
        let renamed = !numbered && base_segment(segment(a)) != base_segment(segment(b));
        let own = node.change.own;
        let access_changed = a.access != b.access;
        if a.hashes.text == b.hashes.text
            && own.is_zero()
            && node.children.is_empty()
            && !renamed
            && !access_changed
        {
            return None;
        }

        let mut kinds = change_kinds(a, b);
        if own.is_zero() && !node.children.is_empty() {
            // Nothing changed in the scope's own lines: it only holds
            // changed scopes. What changed is said on those.
            kinds.retain(|kind| matches!(kind, ChangeKind::Signature | ChangeKind::Access));
        }
        let nothing_to_show = own.is_zero()
            && node.children.is_empty()
            && !renamed
            && !kinds.contains(&ChangeKind::Access);
        if nothing_to_show && self.options.ignore_whitespace {
            // The texts differ only in whitespace, which was asked to be ignored.
            return None;
        }

        node.change.kinds = kinds;
        if renamed {
            node.change.state = ChangeState::Renamed;
            node.change.old_label = Some(segment(a).to_owned());
        }
        Some(node)
    }

    /// Everything under a pair of scopes (or under one that exists on one
    /// side only), in display order.
    fn children(&self, old: Option<usize>, new: Option<usize>) -> Vec<Node> {
        let mut nodes = Vec::new();

        if let Some(new) = new {
            for &child in &self.matcher.new.children[new] {
                let node = match self.matcher.new_to_old[child] {
                    Some(old_child) if self.matcher.same_place(old_child, child) => {
                        self.pair(old_child, child)
                    }
                    Some(old_child) => Some(self.moved(old_child, child, ChangeState::MovedIn)),
                    None => Some(self.one_sided(None, Some(child))),
                };
                nodes.extend(node);
            }
        }

        if let Some(old) = old {
            // Where on the head side a removed scope used to follow.
            let mut anchor = new.map_or(0, |new| self.new_scope(new).range.start_byte);
            for &child in &self.matcher.old.children[old] {
                let mut node = match self.matcher.old_to_new[child] {
                    Some(new_child) if self.matcher.same_place(child, new_child) => {
                        anchor = self.new_scope(new_child).range.end_byte;
                        continue;
                    }
                    Some(new_child) => self.moved(child, new_child, ChangeState::MovedOut),
                    None => self.one_sided(Some(child), None),
                };
                node.order = (anchor, 1, self.old(child).range.start_byte);
                nodes.push(node);
            }
        }

        nodes.sort_by_key(|node| node.order);
        nodes
    }

    /// A scope that exists on one side only, with everything inside it.
    fn one_sided(&self, old: Option<usize>, new: Option<usize>) -> Node {
        let mut node = self.leaf(old, new);
        node.change.state = if new.is_some() {
            ChangeState::Added
        } else {
            ChangeState::Removed
        };
        node.children = self.children(old, new);
        node
    }

    /// One end of a move. A moved scope is listed whole: its lines are all
    /// its own, and what changed inside is told by `moved_delta`.
    fn moved(&self, old: usize, new: usize, state: ChangeState) -> Node {
        let (a, b) = (self.old(old), self.new_scope(new));
        let mut node = match state {
            ChangeState::MovedIn => self.leaf(None, Some(new)),
            _ => self.leaf(Some(old), None),
        };
        node.change.state = state;
        node.change.kinds = change_kinds(a, b);
        node.change.fingerprint = fingerprint(Some(a), Some(b));
        match state {
            ChangeState::MovedIn => {
                node.change.counterpart = Some(a.id.clone());
                // Where the old text is, so a before and after can be shown.
                node.change.old_path = Some(a.path.file.clone());
                node.change.old_range = Some(a.range);
                node.change.own.added = subtree_sum(self.matcher.new, new, self.added);
                node.change.moved_delta = Some(line_delta(
                    self.matcher.old.text(old),
                    self.matcher.new.text(new),
                    self.options.ignore_whitespace,
                ));
            }
            _ => {
                node.change.counterpart = Some(b.id.clone());
                node.change.new_path = Some(b.path.file.clone());
                node.change.new_range = Some(b.range);
                node.change.own.removed = subtree_sum(self.matcher.old, old, self.removed);
            }
        }
        node
    }

    /// A change for one scope, with its own line counts and no children.
    fn leaf(&self, old: Option<usize>, new: Option<usize>) -> Node {
        let (a, b) = (
            old.map(|old| self.old(old)),
            new.map(|new| self.new_scope(new)),
        );
        let shown = b.or(a).expect("a change has a scope on at least one side");
        let own = LineCount {
            added: new.map_or(0, |new| self.added[new]),
            removed: old.map_or(0, |old| self.removed[old]),
        };
        let order = match b {
            Some(scope) => (scope.range.start_byte, 0, 0),
            None => (0, 1, shown.range.start_byte),
        };
        Node {
            change: Change {
                state: ChangeState::Modified,
                kinds: Vec::new(),
                kind: shown.kind,
                lang_kind: shown.lang_kind.clone(),
                label: segment(shown).to_owned(),
                old_label: None,
                name: shown.name.clone(),
                access: shown.access,
                old_id: a.map(|scope| scope.id.clone()),
                new_id: b.map(|scope| scope.id.clone()),
                counterpart: None,
                old_path: a.map(|scope| scope.path.file.clone()),
                new_path: b.map(|scope| scope.path.file.clone()),
                own,
                total: own,
                moved_delta: None,
                fingerprint: fingerprint(a, b),
                old_range: a.map(|scope| scope.range),
                new_range: b.map(|scope| scope.range),
                parent: None,
                children: Vec::new(),
            },
            children: Vec::new(),
            order,
        }
    }
}

fn segment(scope: &Scope) -> &str {
    scope
        .path
        .segments
        .last()
        .map(String::as_str)
        .unwrap_or(scope.path.file.as_str())
}

/// What differs between two versions of a scope, read off their hashes.
fn change_kinds(old: &Scope, new: &Scope) -> Vec<ChangeKind> {
    let (a, b) = (&old.hashes, &new.hashes);
    let mut kinds = Vec::new();
    if a.text != b.text {
        if a.tokens == b.tokens {
            kinds.push(ChangeKind::Formatting);
        } else if a.code == b.code {
            kinds.push(ChangeKind::Comments);
        } else {
            // An import or a block has no signature to speak of.
            let has_signature = !matches!(old.kind, ScopeKind::Import | ScopeKind::Block);
            if a.signature != b.signature && has_signature {
                kinds.push(ChangeKind::Signature);
            }
            if a.body != b.body || (kinds.is_empty() && has_signature) {
                kinds.push(ChangeKind::Body);
            }
        }
    }
    if old.access != new.access {
        kinds.push(ChangeKind::Access);
    }
    kinds.sort();
    kinds
}

/// A hash of the scope's text on both sides. See `Change::fingerprint`.
fn fingerprint(old: Option<&Scope>, new: Option<&Scope>) -> String {
    fn text_hash(scope: Option<&Scope>) -> &str {
        scope.map_or("-", |scope| scope.hashes.text.as_str())
    }
    let joined = format!("{}:{}", text_hash(old), text_hash(new));
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(joined.as_bytes()))
}

fn subtree_sum(side: &Side, scope: usize, counts: &[u32]) -> u32 {
    counts[scope]
        + side.children[scope]
            .iter()
            .map(|child| subtree_sum(side, *child, counts))
            .sum::<u32>()
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<String, Directory>,
    files: Vec<Node>,
}

fn directory_node(directory: Directory, id: String, label: String) -> Node {
    let mut children: Vec<(String, Node)> = Vec::new();
    for (name, child) in directory.directories {
        let child_id = if id == "." {
            name.clone()
        } else {
            format!("{id}/{name}")
        };
        children.push((name.clone(), directory_node(child, child_id, name)));
    }
    for file in directory.files {
        children.push((file.change.label.clone(), file));
    }
    // Directories and files share one name-sorted order, as in the scope tree.
    children.sort_by(|a, b| a.0.cmp(&b.0));

    Node {
        change: Change {
            state: ChangeState::Modified,
            kinds: Vec::new(),
            kind: ScopeKind::Directory,
            lang_kind: String::from("directory"),
            label,
            old_label: None,
            name: None,
            access: Access::Public,
            old_id: Some(id.clone()),
            new_id: Some(id),
            counterpart: None,
            old_path: None,
            new_path: None,
            own: LineCount::default(),
            total: LineCount::default(),
            moved_delta: None,
            fingerprint: String::new(),
            old_range: None,
            new_range: None,
            parent: None,
            children: Vec::new(),
        },
        children: children.into_iter().map(|(_, node)| node).collect(),
        order: (0, 0, 0),
    }
}

/// Writes a node and everything under it in pre-order, filling in parent
/// and child positions and the rolled-up totals. Returns the node's total.
fn flatten(node: Node, parent: Option<u32>, out: &mut Vec<Change>) -> (u32, LineCount) {
    let position = out.len() as u32;
    let mut change = node.change;
    change.parent = parent;
    let mut total = change.own;
    out.push(change);

    let mut children = Vec::new();
    for child in node.children {
        let (child_position, child_total) = flatten(child, Some(position), out);
        children.push(child_position);
        total += child_total;
    }
    let change = &mut out[position as usize];
    change.children = children;
    change.total = total;
    (position, total)
}
