//! Operations on the reference graph that a map needs: lifting references
//! to the level being shown, marking the wires that close loops, bridging
//! over hidden nodes, and finding where a container is entered.

use std::collections::{BTreeMap, HashMap, HashSet};

use mapdiff_core::{Reference, ScopeKind, Tier, Wire, WireKind};
use serde::{Deserialize, Serialize};

use crate::Graph;
use crate::index::Index;

/// How far down the tree a view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Directories. A file stands for itself only when it sits in the root.
    Architecture,
    /// Files.
    Package,
    /// The top two levels inside each file: types and functions, and the
    /// members of types. Imports and loose statements count as their file.
    Module,
    /// Every scope that is not inside a function. A function is one node,
    /// with the named functions nested in it.
    Code,
}

/// What a view shows.
#[derive(Debug, Clone)]
pub struct View {
    cut: Cut,
    /// Scopes that are shown as absent, with everything inside them. Wires
    /// that would pass through them become bridges.
    pub hidden: HashSet<String>,
}

#[derive(Debug, Clone)]
enum Cut {
    Level(Level),
    /// Exactly these scopes: each reference end is lifted to the nearest
    /// one of them at or above it, and dropped when there is none.
    Scopes(HashSet<String>),
}

impl View {
    pub fn level(level: Level) -> Self {
        Self {
            cut: Cut::Level(level),
            hidden: HashSet::new(),
        }
    }

    pub fn scopes(scopes: impl IntoIterator<Item = String>) -> Self {
        Self {
            cut: Cut::Scopes(scopes.into_iter().collect()),
            hidden: HashSet::new(),
        }
    }

    pub fn hiding(mut self, hidden: impl IntoIterator<Item = String>) -> Self {
        self.hidden = hidden.into_iter().collect();
        self
    }
}

/// A child of a container that is used from outside the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPoint {
    pub id: String,
    /// References arriving from outside the container.
    pub incoming: u32,
    /// How many different nodes of the view they arrive from.
    pub from: u32,
}

/// A child of a container that uses something outside the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitPoint {
    pub id: String,
    /// References leaving the container.
    pub outgoing: u32,
    /// How many different nodes of the view they go to.
    pub to: u32,
}

/// The wires among the children of one container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerWires {
    /// Between direct children, so a layout of the container can use them
    /// as they are. Wires that close a loop are marked `Cycle`; leaving
    /// those out leaves no loop.
    pub wires: Vec<Wire>,
    pub entries: Vec<EntryPoint>,
    pub exits: Vec<ExitPoint>,
}

/// A reference graph lifted to a view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lifted {
    /// Between the nodes the view shows, wherever they sit in the tree.
    pub wires: Vec<Wire>,
    /// The same references, per container, between its direct children.
    pub containers: BTreeMap<String, ContainerWires>,
}

/// How many references a wire keeps as examples.
const SAMPLES: usize = 3;

impl Graph {
    /// Lifts every reference to the nodes a view shows.
    pub fn lift(&self, view: &View) -> Lifted {
        let index = Index::new(&self.tree);
        let position_of: HashMap<&str, usize> = index
            .scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), position))
            .collect();
        let hidden: HashSet<usize> = view
            .hidden
            .iter()
            .filter_map(|id| position_of.get(id.as_str()).copied())
            .collect();
        let shown: Option<HashSet<usize>> = match &view.cut {
            Cut::Level(_) => None,
            Cut::Scopes(scopes) => Some(
                scopes
                    .iter()
                    .filter_map(|id| position_of.get(id.as_str()).copied())
                    .collect(),
            ),
        };

        // The node of the view a scope belongs to.
        let node_of = |scope: usize| -> Option<usize> {
            match (&view.cut, &shown) {
                (Cut::Level(level), _) => Some(at_level(&index, scope, *level)),
                (Cut::Scopes(_), Some(shown)) => {
                    let mut current = Some(scope);
                    while let Some(candidate) = current {
                        if shown.contains(&candidate) {
                            return Some(candidate);
                        }
                        current = index.parent[candidate];
                    }
                    None
                }
                (Cut::Scopes(_), None) => None,
            }
        };
        let is_hidden = |node: usize| {
            let mut current = Some(node);
            while let Some(scope) = current {
                if hidden.contains(&scope) {
                    return true;
                }
                current = index.parent[scope];
            }
            false
        };

        // Lift, keeping references in their sorted order so that counts and
        // samples never depend on anything but the graph.
        let mut lifted: BTreeMap<(usize, usize), Vec<&Reference>> = BTreeMap::new();
        for reference in &self.references.references {
            let (Some(from), Some(to)) = (
                position_of.get(reference.from.as_str()),
                position_of.get(reference.to.as_str()),
            ) else {
                continue;
            };
            let (Some(from), Some(to)) = (node_of(*from), node_of(*to)) else {
                continue;
            };
            // Inside one node, or between a node and what it contains,
            // there is nothing to draw.
            if index.is_inside(from, to) || index.is_inside(to, from) {
                continue;
            }
            lifted.entry((from, to)).or_default().push(reference);
        }

        // Bridges: from each shown node, through hidden nodes only, to the
        // shown nodes on the other side.
        let hidden_nodes: HashSet<usize> = lifted
            .keys()
            .flat_map(|(from, to)| [*from, *to])
            .filter(|node| is_hidden(*node))
            .collect();
        let mut successors: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (from, to) in lifted.keys() {
            successors.entry(*from).or_default().push(*to);
        }
        let mut bridges: BTreeMap<(usize, usize), u32> = BTreeMap::new();
        for ((from, first), references) in &lifted {
            if hidden_nodes.contains(from) || !hidden_nodes.contains(first) {
                continue;
            }
            let mut seen = HashSet::from([*first]);
            let mut pending = vec![*first];
            while let Some(node) = pending.pop() {
                for next in successors.get(&node).into_iter().flatten() {
                    if hidden_nodes.contains(next) {
                        if seen.insert(*next) {
                            pending.push(*next);
                        }
                    } else if next != from && !lifted.contains_key(&(*from, *next)) {
                        *bridges.entry((*from, *next)).or_insert(0) += references.len() as u32;
                    }
                }
            }
        }

        let id = |scope: usize| index.scopes[scope].id.clone();
        let mut out = Lifted::default();
        // Wires between children of each container, before cycle marking.
        // Bridges are kept apart: between two children that already have
        // references of their own, a path through hidden scopes adds
        // nothing, and must not be counted with them.
        type Slots = BTreeMap<usize, BTreeMap<(usize, usize), Wire>>;
        let mut inside: Slots = BTreeMap::new();
        let mut bridged: Slots = BTreeMap::new();
        type Doors = BTreeMap<usize, BTreeMap<usize, (u32, HashSet<usize>)>>;
        let mut entries: Doors = BTreeMap::new();
        let mut exits: Doors = BTreeMap::new();

        let mut place = |from: usize, to: usize, wire: &Wire| {
            let (container, from_child, to_child) = siblings(&index, from, to);
            let slots = if wire.kind == WireKind::Bridge {
                &mut bridged
            } else {
                &mut inside
            };
            let slot = slots
                .entry(container)
                .or_default()
                .entry((from_child, to_child))
                .or_insert_with(|| Wire {
                    from: id(from_child),
                    to: id(to_child),
                    kind: wire.kind,
                    count: 0,
                    precise: 0,
                    kinds: BTreeMap::new(),
                    samples: Vec::new(),
                });
            absorb(slot, wire);
            if wire.kind == WireKind::Bridge {
                return;
            }

            // Every container that holds the target but not the source is
            // entered by these references, at its child on the way to the
            // target.
            let mut child = to;
            let mut current = index.parent[to];
            while let Some(scope) = current {
                if index.is_inside(from, scope) {
                    break;
                }
                let entry = entries.entry(scope).or_default().entry(child).or_default();
                entry.0 += wire.count;
                entry.1.insert(from);
                child = scope;
                current = index.parent[scope];
            }
            // And every container that holds the source but not the target
            // is left by them, at its child on the way to the source.
            let mut child = from;
            let mut current = index.parent[from];
            while let Some(scope) = current {
                if index.is_inside(to, scope) {
                    break;
                }
                let exit = exits.entry(scope).or_default().entry(child).or_default();
                exit.0 += wire.count;
                exit.1.insert(to);
                child = scope;
                current = index.parent[scope];
            }
        };

        for ((from, to), references) in &lifted {
            if hidden_nodes.contains(from) || hidden_nodes.contains(to) {
                continue;
            }
            let wire = wire_of(id(*from), id(*to), references);
            place(*from, *to, &wire);
            out.wires.push(wire);
        }
        for ((from, to), count) in &bridges {
            let wire = Wire {
                from: id(*from),
                to: id(*to),
                kind: WireKind::Bridge,
                count: *count,
                precise: 0,
                kinds: BTreeMap::new(),
                samples: Vec::new(),
            };
            place(*from, *to, &wire);
            out.wires.push(wire);
        }
        out.wires
            .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));

        let mut containers: BTreeMap<usize, ContainerWires> = BTreeMap::new();
        for (container, bridges) in bridged {
            let wires = inside.entry(container).or_default();
            for (ends, bridge) in bridges {
                wires.entry(ends).or_insert(bridge);
            }
        }
        for (container, wires) in inside {
            let mut wires: Vec<((usize, usize), Wire)> = wires.into_iter().collect();
            mark_cycles(&mut wires);
            containers.entry(container).or_default().wires =
                wires.into_iter().map(|(_, wire)| wire).collect();
        }
        for (container, children) in entries {
            let mut list: Vec<EntryPoint> = children
                .into_iter()
                .map(|(child, (incoming, from))| EntryPoint {
                    id: id(child),
                    incoming,
                    from: from.len() as u32,
                })
                .collect();
            // Most used first; the ID settles ties.
            list.sort_by(|a, b| b.incoming.cmp(&a.incoming).then(a.id.cmp(&b.id)));
            containers.entry(container).or_default().entries = list;
        }
        for (container, children) in exits {
            let mut list: Vec<ExitPoint> = children
                .into_iter()
                .map(|(child, (outgoing, to))| ExitPoint {
                    id: id(child),
                    outgoing,
                    to: to.len() as u32,
                })
                .collect();
            list.sort_by(|a, b| b.outgoing.cmp(&a.outgoing).then(a.id.cmp(&b.id)));
            containers.entry(container).or_default().exits = list;
        }
        out.containers = containers
            .into_iter()
            .map(|(container, wires)| (id(container), wires))
            .collect();
        out
    }
}

/// Answers which box of a level a scope is drawn in.
pub struct Lifter<'a> {
    index: Index<'a>,
    positions: HashMap<&'a str, usize>,
}

impl Graph {
    pub fn lifter(&self) -> Lifter<'_> {
        let index = Index::new(&self.tree);
        let positions = index
            .scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), position))
            .collect();
        Lifter { index, positions }
    }
}

impl<'a> Lifter<'a> {
    /// The node of the level that the scope is, or is part of.
    pub fn at(&self, id: &str, level: Level) -> Option<&'a str> {
        let scope = *self.positions.get(id)?;
        Some(
            self.index.scopes[at_level(&self.index, scope, level)]
                .id
                .as_str(),
        )
    }

    /// The first level, from the top, at which the scope is a node of its
    /// own. A scope inside a function never is: it gets `Code`.
    pub fn shallowest(&self, id: &str) -> Option<Level> {
        let scope = *self.positions.get(id)?;
        [Level::Architecture, Level::Package, Level::Module]
            .into_iter()
            .find(|level| at_level(&self.index, scope, *level) == scope)
            .or(Some(Level::Code))
    }
}

/// The node of a level that a scope belongs to.
pub(crate) fn at_level(index: &Index<'_>, scope: usize, level: Level) -> usize {
    let file = index.file_of[scope];
    match level {
        Level::Code => {
            // The outermost function around it, when there is one.
            let mut node = scope;
            let mut current = Some(scope);
            while let Some(candidate) = current {
                if matches!(
                    index.kind(candidate),
                    ScopeKind::Function | ScopeKind::Method | ScopeKind::Constructor
                ) {
                    node = candidate;
                }
                current = index.parent[candidate];
            }
            node
        }
        Level::Package => file,
        Level::Module => {
            let mut current = at_level(index, scope, Level::Code);
            while index.scopes[current].path.segments.len() > 2
                || matches!(index.kind(current), ScopeKind::Import | ScopeKind::Block)
            {
                match index.parent[current] {
                    Some(parent) => current = parent,
                    None => break,
                }
            }
            current
        }
        Level::Architecture => {
            if index.kind(scope) == ScopeKind::Directory {
                return scope;
            }
            match index.parent[file] {
                // A file in the root has no directory of its own to stand in.
                Some(directory) if index.parent[directory].is_some() => directory,
                _ => file,
            }
        }
    }
}

/// The container that holds both nodes, and its two children on the way to
/// each. The nodes must not contain one another.
fn siblings(index: &Index<'_>, from: usize, to: usize) -> (usize, usize, usize) {
    let chain = |scope: usize| {
        let mut chain = vec![scope];
        let mut current = index.parent[scope];
        while let Some(parent) = current {
            chain.push(parent);
            current = index.parent[parent];
        }
        chain.reverse();
        chain
    };
    let (from_chain, to_chain) = (chain(from), chain(to));
    let mut shared = 0;
    while shared < from_chain.len()
        && shared < to_chain.len()
        && from_chain[shared] == to_chain[shared]
    {
        shared += 1;
    }
    // Both chains start at the root and neither node contains the other, so
    // there is a shared ancestor and each chain goes on below it.
    (from_chain[shared - 1], from_chain[shared], to_chain[shared])
}

fn wire_of(from: String, to: String, references: &[&Reference]) -> Wire {
    let mut kinds = BTreeMap::new();
    for reference in references {
        *kinds.entry(reference.kind).or_insert(0) += 1;
    }
    Wire {
        from,
        to,
        kind: WireKind::Direct,
        count: references.len() as u32,
        precise: references
            .iter()
            .filter(|reference| reference.tier == Tier::Precise)
            .count() as u32,
        kinds,
        samples: references
            .iter()
            .take(SAMPLES)
            .map(|reference| (*reference).clone())
            .collect(),
    }
}

/// Adds one wire's references to a wire between two ancestors.
fn absorb(into: &mut Wire, wire: &Wire) {
    into.count += wire.count;
    into.precise += wire.precise;
    for (kind, count) in &wire.kinds {
        *into.kinds.entry(*kind).or_insert(0) += count;
    }
    for sample in &wire.samples {
        if into.samples.len() < SAMPLES {
            into.samples.push(sample.clone());
        }
    }
}

/// Marks as `Cycle` the wires that close a loop among siblings, so that the
/// wires left over have no loop and a layout can flow one way.
///
/// The rule is fixed. Within each group of nodes that reach each other, the
/// nodes are put in a line: a node nothing left uses goes to the front, a
/// node that uses nothing left goes to the back, and otherwise the node
/// that uses the others most and is used least goes to the front (the
/// ordering of Eades, Lin and Smyth, with reference counts as weights).
/// Wires that point backwards along the line are candidates, and the
/// heaviest candidates are let off while doing so opens no loop. What is
/// left are few, light wires, each of which really closes a loop. Position
/// in the tree settles every tie, so the same graph always gives the same
/// marks.
pub(crate) fn mark_cycles(wires: &mut [((usize, usize), Wire)]) {
    let mut nodes: Vec<usize> = wires
        .iter()
        .flat_map(|((from, to), _)| [*from, *to])
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
    for ((from, to), _) in wires.iter() {
        successors.entry(*from).or_default().push(*to);
    }
    for targets in successors.values_mut() {
        targets.sort_unstable();
    }

    let mut back: HashSet<(usize, usize)> = HashSet::new();
    for component in strongly_connected(&nodes, &successors) {
        if component.len() < 2 {
            continue;
        }
        let members: HashSet<usize> = component.iter().copied().collect();
        let inside: Vec<(usize, usize, i64)> = wires
            .iter()
            .filter(|((from, to), _)| members.contains(from) && members.contains(to))
            .map(|((from, to), wire)| (*from, *to, i64::from(wire.count)))
            .collect();

        // The line, built from both ends. `component` is sorted, so the
        // first node that qualifies is the one earliest in the tree.
        let mut remaining = component.clone();
        let mut front: Vec<usize> = Vec::new();
        let mut rear: Vec<usize> = Vec::new();
        while !remaining.is_empty() {
            let left: HashSet<usize> = remaining.iter().copied().collect();
            let mut uses: HashMap<usize, i64> = HashMap::new();
            let mut used: HashMap<usize, i64> = HashMap::new();
            for (from, to, count) in &inside {
                if left.contains(from) && left.contains(to) {
                    *uses.entry(*from).or_insert(0) += count;
                    *used.entry(*to).or_insert(0) += count;
                }
            }
            let uses = |node: &usize| uses.get(node).copied().unwrap_or(0);
            let used = |node: &usize| used.get(node).copied().unwrap_or(0);
            let (chosen, to_front) =
                if let Some(node) = remaining.iter().find(|node| uses(node) == 0) {
                    (*node, false)
                } else if let Some(node) = remaining.iter().find(|node| used(node) == 0) {
                    (*node, true)
                } else {
                    let node = remaining
                        .iter()
                        .min_by_key(|node| (std::cmp::Reverse(uses(node) - used(node)), **node))
                        .expect("the loop runs while nodes remain");
                    (*node, true)
                };
            remaining.retain(|node| *node != chosen);
            if to_front {
                front.push(chosen);
            } else {
                rear.push(chosen);
            }
        }
        let place: HashMap<usize, usize> = front
            .iter()
            .chain(rear.iter().rev())
            .enumerate()
            .map(|(place, node)| (*node, place))
            .collect();

        let mut kept: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut candidates: Vec<(usize, usize, i64)> = Vec::new();
        for (from, to, count) in &inside {
            if place[from] < place[to] {
                kept.entry(*from).or_default().push(*to);
            } else {
                candidates.push((*from, *to, *count));
            }
        }
        // Heaviest first: a wire is let off when its target cannot already
        // get back to its source.
        candidates.sort_by_key(|(from, to, count)| (std::cmp::Reverse(*count), *from, *to));
        for (from, to, _) in candidates {
            if reaches(&kept, to, from) {
                back.insert((from, to));
            } else {
                kept.entry(from).or_default().push(to);
            }
        }
    }
    for (ends, wire) in wires.iter_mut() {
        if back.contains(ends) {
            wire.kind = WireKind::Cycle;
        }
    }
}

/// True when `target` can be reached from `start` along the wires given.
fn reaches(successors: &HashMap<usize, Vec<usize>>, start: usize, target: usize) -> bool {
    let mut seen = HashSet::from([start]);
    let mut pending = vec![start];
    while let Some(node) = pending.pop() {
        if node == target {
            return true;
        }
        for next in successors.get(&node).into_iter().flatten() {
            if seen.insert(*next) {
                pending.push(*next);
            }
        }
    }
    false
}

/// Groups of nodes that can all reach each other (Tarjan's algorithm,
/// without recursion). Nodes and groups come out in a fixed order.
fn strongly_connected(nodes: &[usize], successors: &HashMap<usize, Vec<usize>>) -> Vec<Vec<usize>> {
    let mut number: HashMap<usize, usize> = HashMap::new();
    let mut low: HashMap<usize, usize> = HashMap::new();
    let mut on_stack: HashSet<usize> = HashSet::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut components = Vec::new();
    let mut counter = 0;
    let no_targets: Vec<usize> = Vec::new();

    for root in nodes {
        if number.contains_key(root) {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(*root, 0)];
        while let Some((node, next)) = work.last().copied() {
            if next == 0 {
                number.insert(node, counter);
                low.insert(node, counter);
                counter += 1;
                stack.push(node);
                on_stack.insert(node);
            }
            let targets = successors.get(&node).unwrap_or(&no_targets);
            if let Some(target) = targets.get(next).copied() {
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                if !number.contains_key(&target) {
                    work.push((target, 0));
                } else if on_stack.contains(&target) {
                    let reached = number[&target].min(low[&node]);
                    low.insert(node, reached);
                }
            } else {
                work.pop();
                if let Some((parent, _)) = work.last() {
                    let reached = low[&node].min(low[parent]);
                    low.insert(*parent, reached);
                }
                if low[&node] == number[&node] {
                    let mut component = Vec::new();
                    while let Some(member) = stack.pop() {
                        on_stack.remove(&member);
                        component.push(member);
                        if member == node {
                            break;
                        }
                    }
                    component.sort_unstable();
                    components.push(component);
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(count: u32) -> Wire {
        Wire {
            from: String::new(),
            to: String::new(),
            kind: WireKind::Direct,
            count,
            precise: 0,
            kinds: BTreeMap::new(),
            samples: Vec::new(),
        }
    }

    fn cycles(edges: &[(usize, usize, u32)]) -> Vec<(usize, usize)> {
        let mut wires: Vec<((usize, usize), Wire)> = edges
            .iter()
            .map(|(from, to, count)| ((*from, *to), wire(*count)))
            .collect();
        mark_cycles(&mut wires);
        wires
            .iter()
            .filter(|(_, wire)| wire.kind == WireKind::Cycle)
            .map(|(ends, _)| *ends)
            .collect()
    }

    /// True when the wires not marked as cycles have no loop.
    fn is_acyclic(edges: &[(usize, usize, u32)], marked: &[(usize, usize)]) -> bool {
        let kept: Vec<(usize, usize)> = edges
            .iter()
            .map(|(from, to, _)| (*from, *to))
            .filter(|ends| !marked.contains(ends))
            .collect();
        let mut nodes: Vec<usize> = kept.iter().flat_map(|(from, to)| [*from, *to]).collect();
        nodes.sort_unstable();
        nodes.dedup();
        let mut successors: HashMap<usize, Vec<usize>> = HashMap::new();
        for (from, to) in &kept {
            successors.entry(*from).or_default().push(*to);
        }
        strongly_connected(&nodes, &successors)
            .iter()
            .all(|component| component.len() == 1)
    }

    #[test]
    fn a_chain_has_no_cycle_wires() {
        assert!(cycles(&[(1, 2, 1), (2, 3, 1), (1, 3, 1)]).is_empty());
    }

    #[test]
    fn of_two_wires_between_a_pair_the_lighter_one_is_the_cycle() {
        // 1 uses 2 five times; 2 uses 1 once. The one use is the back wire.
        assert_eq!(cycles(&[(1, 2, 5), (2, 1, 1)]), [(2, 1)]);
        assert_eq!(cycles(&[(1, 2, 1), (2, 1, 5)]), [(1, 2)]);
        // Equal weights: position decides, the same way every time.
        assert_eq!(cycles(&[(1, 2, 3), (2, 1, 3)]), [(2, 1)]);
    }

    #[test]
    fn a_node_everything_uses_keeps_its_incoming_wires() {
        // 9 is used heavily by 1, 2 and 3 and uses each of them once, the
        // way a crate's root file is used by, and re-exports, its modules.
        let edges = [
            (1, 9, 30),
            (2, 9, 20),
            (3, 9, 50),
            (9, 1, 1),
            (9, 2, 1),
            (9, 3, 1),
            (1, 2, 4),
        ];
        assert_eq!(cycles(&edges), [(9, 1), (9, 2), (9, 3)]);
    }

    #[test]
    fn a_backward_wire_that_closes_no_loop_is_not_marked() {
        // Two loops share node 2. Breaking 3 -> 1 and 4 -> 2 is enough.
        let edges = [
            (1, 2, 5),
            (2, 3, 5),
            (3, 1, 1),
            (2, 4, 5),
            (4, 2, 1),
            (4, 3, 2),
        ];
        let marked = cycles(&edges);
        assert!(is_acyclic(&edges, &marked));
        assert_eq!(marked.len(), 2, "{marked:?}");
    }

    #[test]
    fn marking_always_leaves_no_loop() {
        let graphs: [&[(usize, usize, u32)]; 4] = [
            &[(1, 2, 1), (2, 3, 1), (3, 1, 1)],
            &[
                (1, 2, 2),
                (2, 3, 2),
                (3, 1, 2),
                (2, 1, 1),
                (3, 2, 1),
                (1, 3, 9),
            ],
            &[
                (1, 2, 1),
                (2, 1, 1),
                (2, 3, 1),
                (3, 4, 1),
                (4, 2, 1),
                (4, 5, 1),
            ],
            &[
                (5, 4, 1),
                (4, 3, 1),
                (3, 5, 1),
                (3, 2, 7),
                (2, 3, 7),
                (1, 5, 1),
            ],
        ];
        for edges in graphs {
            let marked = cycles(edges);
            assert!(!marked.is_empty(), "{edges:?} has a loop to break");
            assert!(
                is_acyclic(edges, &marked),
                "{edges:?} still loops after {marked:?}"
            );
            assert_eq!(marked, cycles(edges), "marks must not vary between runs");
        }
    }

    #[test]
    fn groups_that_reach_each_other_are_found() {
        let successors: HashMap<usize, Vec<usize>> = HashMap::from([
            (1, vec![2]),
            (2, vec![3]),
            (3, vec![1, 4]),
            (4, vec![5]),
            (5, vec![4]),
        ]);
        let mut components = strongly_connected(&[1, 2, 3, 4, 5, 6], &successors);
        components.sort();
        assert_eq!(components, [vec![1, 2, 3], vec![4, 5], vec![6]]);
    }
}
