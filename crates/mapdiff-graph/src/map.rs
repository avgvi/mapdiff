//! What a map draws: the scopes shown at a level, with what a layout needs
//! to size them, and the wires among the children of each container.
//!
//! For a comparison, `union` merges the map before a change with the map
//! after it, so that one layout serves both sides.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use mapdiff_core::{ChangeSet, ChangeState, SCHEMA_VERSION, ScopeKind, Wire, WireKind};
use serde::{Deserialize, Serialize};

use crate::Graph;
use crate::index::Index;
use crate::ops::{EntryPoint, ExitPoint, Level, View, at_level, mark_cycles};

/// Which side of a comparison something exists on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    #[default]
    Both,
    /// Only after the change.
    Added,
    /// Only before the change. A map keeps its place, as a ghost.
    Removed,
}

/// One box on the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapNode {
    pub id: String,
    pub parent: Option<String>,
    pub kind: ScopeKind,
    pub lang_kind: String,
    /// What the box is titled: the scope's name, or the first line of its
    /// text when it has none.
    pub label: String,
    /// The line the scope starts on, counted from 1. 0 for a directory.
    pub line: u32,
    /// How many lines of source the scope spans. 0 for a directory.
    pub lines: u32,
    /// The length of its longest line, with a tab counted as four.
    pub columns: u32,
    /// True for a scope that is part of its parent's box at this level and
    /// only drawn as a small mark in it: a file, at the level of
    /// directories.
    pub minor: bool,
    pub presence: Presence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapWire {
    #[serde(flatten)]
    pub wire: Wire,
    pub presence: Presence,
}

/// The wires among the children of one container, and its doors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapContainer {
    pub wires: Vec<MapWire>,
    /// Children used from outside the container.
    pub entries: Vec<EntryPoint>,
    /// Children that use something outside it.
    pub exits: Vec<ExitPoint>,
}

/// Why an unchanged box is affected by a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cause {
    /// The box of the changed scope it uses.
    pub at: String,
    /// The changed scope's label, and how it changed.
    pub label: String,
    pub reasons: Vec<String>,
}

/// A change, drawn on its map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay {
    /// For each box, the positions in the change set of the changes drawn
    /// in it: its own, and those of scopes inside it that this level does
    /// not show.
    pub changes: BTreeMap<String, Vec<u32>>,
    /// Boxes that did not change and use, or are used by, a scope that did.
    pub near: Vec<String>,
    /// Boxes that did not change and use a scope whose change they can
    /// see: it was removed, renamed, moved, or its signature or access
    /// changed.
    pub impacted: BTreeMap<String, Vec<Cause>>,
    /// The changes in an order to read them in: what is used before what
    /// uses it, with the changes of one file together. Code first, then
    /// tests, then prose.
    pub order: Vec<u32>,
    /// The changes that are to tests. Tests are not on the map, so these
    /// have no box: they are only listed.
    pub tests: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapData {
    pub schema_version: u32,
    pub level: Level,
    /// In pre-order: a parent before its children, siblings in source
    /// order.
    pub nodes: Vec<MapNode>,
    pub containers: BTreeMap<String, MapContainer>,
    /// Set on the map of a comparison.
    pub overlay: Option<Overlay>,
}

/// A tab is drawn four columns wide.
const TAB: u32 = 4;
/// Labels taken from source text are cut to this many characters.
const LABEL: usize = 60;

impl Graph {
    /// The map of the repository at a level. `root` is where the files
    /// are, to measure their lines.
    pub fn map(&self, root: &Path, level: Level, hidden: &HashSet<String>) -> MapData {
        let index = Index::new(&self.tree);
        let concealed = |scope: usize| {
            let mut current = Some(scope);
            while let Some(candidate) = current {
                if hidden.contains(&index.scopes[candidate].id) {
                    return true;
                }
                current = index.parent[candidate];
            }
            false
        };

        // The nodes of the level, and every container above them.
        let mut shown = vec![false; index.scopes.len()];
        for scope in 0..index.scopes.len() {
            if concealed(scope) {
                continue;
            }
            let mut current = Some(at_level(&index, scope, level));
            while let Some(node) = current {
                if shown[node] {
                    break;
                }
                shown[node] = true;
                current = index.parent[node];
            }
        }

        // At the level of directories a file is part of its directory, but
        // still drawn in it, small.
        let mut minor = vec![false; index.scopes.len()];
        if level == Level::Architecture {
            for (position, scope) in index.scopes.iter().enumerate() {
                let is_file = matches!(scope.kind, ScopeKind::File | ScopeKind::Document);
                let in_shown = index.parent[position].is_some_and(|parent| shown[parent]);
                if is_file && !shown[position] && in_shown && !concealed(position) {
                    shown[position] = true;
                    minor[position] = true;
                }
            }
        }

        // Line lengths, one file at a time.
        let mut nodes = Vec::new();
        let mut lengths: Vec<u32> = Vec::new();
        let mut first_lines: Vec<String> = Vec::new();
        let mut measured: Option<usize> = None;
        for (position, scope) in index.scopes.iter().enumerate() {
            if !shown[position] {
                continue;
            }
            let file = index.file_of[position];
            let is_directory = scope.kind == ScopeKind::Directory;
            if !is_directory && measured != Some(file) {
                let text = std::fs::read_to_string(root.join(&index.scopes[file].path.file))
                    .unwrap_or_default();
                lengths.clear();
                first_lines.clear();
                for line in text.lines() {
                    lengths.push(
                        line.chars()
                            .map(|character| if character == '\t' { TAB } else { 1 })
                            .sum(),
                    );
                    first_lines.push(line.trim().chars().take(LABEL).collect());
                }
                measured = Some(file);
            }
            let (line, lines, columns, first_line) = if is_directory {
                (0, 0, 0, None)
            } else {
                let start = scope.range.start.line as usize;
                // A range that ends at the start of a line does not
                // include that line.
                let end = match scope.range.end.column {
                    0 if scope.range.end.line as usize > start => scope.range.end.line as usize,
                    _ => scope.range.end.line as usize + 1,
                }
                .min(lengths.len());
                let start = start.min(end);
                (
                    start as u32 + 1,
                    (end - start) as u32,
                    lengths[start..end].iter().copied().max().unwrap_or(0),
                    // The header starts after any comment above the scope.
                    first_lines
                        .get(scope.header_range.start.line as usize)
                        .cloned(),
                )
            };
            let label = match (&scope.name, first_line) {
                (Some(name), _) => name.clone(),
                (None, Some(line)) if !line.is_empty() => line,
                _ => scope
                    .path
                    .segments
                    .last()
                    .cloned()
                    .unwrap_or_else(|| scope.path.file.clone()),
            };
            nodes.push(MapNode {
                id: scope.id.clone(),
                parent: scope.parent.clone(),
                kind: scope.kind,
                lang_kind: scope.lang_kind.clone(),
                label,
                line,
                lines,
                columns,
                minor: minor[position],
                presence: Presence::Both,
            });
        }

        let lifted = self.lift(&View::level(level).hiding(hidden.iter().cloned()));
        let containers = lifted
            .containers
            .into_iter()
            .map(|(id, inside)| {
                let wires = inside
                    .wires
                    .into_iter()
                    .map(|wire| MapWire {
                        wire,
                        presence: Presence::Both,
                    })
                    .collect();
                (
                    id,
                    MapContainer {
                        wires,
                        entries: inside.entries,
                        exits: inside.exits,
                    },
                )
            })
            .collect();
        MapData {
            schema_version: SCHEMA_VERSION,
            level,
            nodes,
            containers,
            overlay: None,
        }
    }
}

/// Merges the map from before a change with the map after it.
///
/// Everything in `head` is kept as it is. A scope that exists only in
/// `base` is added as a ghost (`Presence::Removed`) in the container it was
/// in, after the sibling it used to follow, and a wire that exists only in
/// `base` is added the same way. A layout of the result therefore has a
/// place for everything on either side, and nothing moves when the reader
/// switches between them.
///
/// A scope that moved to another container is on the map twice: a ghost
/// where it was and a box where it is, so that the two can be joined.
pub fn union(base: &MapData, head: &MapData, changes: &ChangeSet) -> MapData {
    union_with_image(base, head, changes).0
}

/// `union`, and for each box of `base` the box of the result that it is.
pub(crate) fn union_with_image(
    base: &MapData,
    head: &MapData,
    changes: &ChangeSet,
) -> (MapData, HashMap<String, String>) {
    let head_position: HashMap<&str, usize> = head
        .nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.id.as_str(), position))
        .collect();
    let mut head_children: HashMap<&str, Vec<usize>> = HashMap::new();
    for (position, node) in head.nodes.iter().enumerate() {
        if let Some(parent) = node.parent.as_deref() {
            head_children.entry(parent).or_default().push(position);
        }
    }

    // Where the change set says a scope went.
    let mut renamed: HashMap<&str, &str> = HashMap::new();
    for change in &changes.changes {
        let Some(old) = change.old_id.as_deref() else {
            continue;
        };
        let new = match change.state {
            // What moved away keeps a ghost at its old place.
            ChangeState::MovedOut | ChangeState::Removed | ChangeState::Added => None,
            _ => change.new_id.as_deref(),
        };
        if let Some(new) = new {
            renamed.insert(old, new);
        }
    }

    let base_ids: HashSet<&str> = base.nodes.iter().map(|node| node.id.as_str()).collect();

    // The node of the merged map that each node of `base` is.
    let mut image: HashMap<&str, String> = HashMap::new();
    let mut taken: HashSet<usize> = HashSet::new();
    let mut ghosts: Vec<MapNode> = Vec::new();
    // For each parent in the merged map: (the sibling a ghost follows, the
    // ghost), in the order they were in before.
    let mut ghosts_under: HashMap<String, Vec<(Option<String>, usize)>> = HashMap::new();
    let mut previous_sibling: HashMap<&str, String> = HashMap::new();

    for node in &base.nodes {
        let parent_image = node
            .parent
            .as_deref()
            .and_then(|parent| image.get(parent))
            .cloned();
        let mut found: Option<usize> = renamed
            .get(node.id.as_str())
            .and_then(|new| head_position.get(new))
            .copied()
            .filter(|position| !taken.contains(position));
        if found.is_none() {
            found = head_position
                .get(node.id.as_str())
                .copied()
                .filter(|position| !taken.contains(position))
                // The same ID under another parent is another scope.
                .filter(|position| head.nodes[*position].parent == parent_image);
        }
        if found.is_none() {
            // Under a parent that was renamed or moved, a child keeps its
            // kind and name but not its ID.
            found = parent_image
                .as_deref()
                .and_then(|parent| head_children.get(parent))
                .and_then(|children| {
                    children.iter().copied().find(|position| {
                        let candidate = &head.nodes[*position];
                        !taken.contains(position)
                            && candidate.kind == node.kind
                            && candidate.label == node.label
                            && !base_ids.contains(candidate.id.as_str())
                    })
                });
        }

        let follows = node
            .parent
            .as_deref()
            .and_then(|parent| previous_sibling.get(parent))
            .cloned();
        let merged_id = match found {
            Some(position) => {
                taken.insert(position);
                head.nodes[position].id.clone()
            }
            None => {
                let id = match head_position.contains_key(node.id.as_str()) {
                    // The ID now names something else.
                    true => format!("{} (before)", node.id),
                    false => node.id.clone(),
                };
                ghosts.push(MapNode {
                    id: id.clone(),
                    parent: parent_image.clone(),
                    presence: Presence::Removed,
                    ..node.clone()
                });
                if let Some(parent) = &parent_image {
                    ghosts_under
                        .entry(parent.clone())
                        .or_default()
                        .push((follows, ghosts.len() - 1));
                }
                id
            }
        };
        // A scope that moved elsewhere is no longer a sibling to follow.
        let stayed = found.is_none_or(|position| head.nodes[position].parent == parent_image);
        if let Some(parent) = node.parent.as_deref()
            && stayed
        {
            previous_sibling.insert(parent, merged_id.clone());
        }
        image.insert(node.id.as_str(), merged_id);
    }

    // The merged nodes, in pre-order, ghosts after the sibling they
    // followed.
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_id: HashMap<String, MapNode> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();
    for (position, node) in head.nodes.iter().enumerate() {
        let mut node = node.clone();
        if !taken.contains(&position) {
            node.presence = Presence::Added;
        }
        match &node.parent {
            Some(parent) => children
                .entry(parent.clone())
                .or_default()
                .push(node.id.clone()),
            None => roots.push(node.id.clone()),
        }
        by_id.insert(node.id.clone(), node);
    }
    for ghost in &ghosts {
        by_id.insert(ghost.id.clone(), ghost.clone());
        if ghost.parent.is_none() {
            roots.push(ghost.id.clone());
        }
    }
    for (parent, list) in &ghosts_under {
        let siblings = children.entry(parent.clone()).or_default();
        for (follows, ghost) in list {
            let id = ghosts[*ghost].id.clone();
            let at = follows
                .as_ref()
                .and_then(|sibling| siblings.iter().position(|other| other == sibling))
                .map_or(0, |position| position + 1);
            siblings.insert(at, id);
        }
    }
    let mut nodes = Vec::with_capacity(by_id.len());
    let mut pending: Vec<String> = roots.into_iter().rev().collect();
    while let Some(id) = pending.pop() {
        if let Some(below) = children.get(&id) {
            pending.extend(below.iter().rev().cloned());
        }
        if let Some(node) = by_id.remove(&id) {
            nodes.push(node);
        }
    }

    // Wires and doors.
    let mut containers = head.containers.clone();
    let parent_of: HashMap<&str, Option<&str>> = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.parent.as_deref()))
        .collect();
    let order: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.id.as_str(), position))
        .collect();
    let mut before: HashSet<(&str, &str, &str)> = HashSet::new();
    for (container, inside) in &base.containers {
        let Some(merged) = image.get(container.as_str()) else {
            continue;
        };
        let into = containers.entry(merged.clone()).or_default();
        let mut gained = false;
        for wire in &inside.wires {
            let (Some(from), Some(to)) = (
                image.get(wire.wire.from.as_str()),
                image.get(wire.wire.to.as_str()),
            ) else {
                continue;
            };
            // A scope that moved to another container took its wires along.
            let sibling = |id: &str| parent_of.get(id).copied().flatten() == Some(merged.as_str());
            if !sibling(from) || !sibling(to) {
                continue;
            }
            before.insert((merged.as_str(), from.as_str(), to.as_str()));
            if into
                .wires
                .iter()
                .any(|known| known.wire.from == *from && known.wire.to == *to)
            {
                continue;
            }
            let mut wire = wire.wire.clone();
            wire.from = from.clone();
            wire.to = to.clone();
            if wire.kind == WireKind::Cycle {
                wire.kind = WireKind::Direct;
            }
            wire.samples.clear();
            into.wires.push(MapWire {
                wire,
                presence: Presence::Removed,
            });
            gained = true;
        }
        for entry in &inside.entries {
            if let Some(id) = image.get(entry.id.as_str())
                && !into.entries.iter().any(|known| known.id == *id)
            {
                into.entries.push(EntryPoint {
                    id: id.clone(),
                    ..entry.clone()
                });
            }
        }
        for exit in &inside.exits {
            if let Some(id) = image.get(exit.id.as_str())
                && !into.exits.iter().any(|known| known.id == *id)
            {
                into.exits.push(ExitPoint {
                    id: id.clone(),
                    ..exit.clone()
                });
            }
        }
        if gained {
            // With more wires there may be more loops: mark them afresh, by
            // the same rule.
            into.wires
                .sort_by(|a, b| (&a.wire.from, &a.wire.to).cmp(&(&b.wire.from, &b.wire.to)));
            let mut marking: Vec<((usize, usize), Wire)> = into
                .wires
                .iter()
                .map(|wire| {
                    let mut plain = wire.wire.clone();
                    if plain.kind == WireKind::Cycle {
                        plain.kind = WireKind::Direct;
                    }
                    let ends = (
                        order
                            .get(plain.from.as_str())
                            .copied()
                            .unwrap_or(usize::MAX),
                        order.get(plain.to.as_str()).copied().unwrap_or(usize::MAX),
                    );
                    (ends, plain)
                })
                .collect();
            mark_cycles(&mut marking);
            for (wire, (_, marked)) in into.wires.iter_mut().zip(marking) {
                wire.wire.kind = marked.kind;
            }
        }
    }
    for (container, inside) in &mut containers {
        for wire in &mut inside.wires {
            if wire.presence == Presence::Both
                && !before.contains(&(
                    container.as_str(),
                    wire.wire.from.as_str(),
                    wire.wire.to.as_str(),
                ))
            {
                wire.presence = Presence::Added;
            }
        }
    }

    let image = image
        .into_iter()
        .map(|(before, merged)| (before.to_owned(), merged))
        .collect();
    (
        MapData {
            schema_version: SCHEMA_VERSION,
            level: head.level,
            nodes,
            containers,
            overlay: None,
        },
        image,
    )
}
