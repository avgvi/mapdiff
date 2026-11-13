//! Graphs and maps of the two sides of a comparison. A side that is not
//! the working tree is written to a temporary directory first, since the
//! graph is built from files on disk.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use mapdiff_core::{Change, ChangeSet, ChangeState, ScopeKind};
use mapdiff_gitdiff::{Comparison, Error, Repo, Side};

use crate::impact::{ImpactOptions, impact};
use crate::map::{Cause, MapData, Overlay, union_with_image};
use crate::ops::Level;
use crate::{Graph, Precise, build_with};

/// One side of a comparison as files on disk, with its graph. A temporary
/// copy is removed when this value goes away.
pub struct Checkout {
    pub root: PathBuf,
    pub graph: Graph,
    _temporary: Option<Temporary>,
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Puts one side of a comparison on disk and builds its graph.
///
/// The working tree is read in place, with `precise` deciding whether an
/// index is used. Any other side is exported and gets the fast tier only:
/// an index covers the files as they are now.
pub fn checkout(repo: &Repo, side: &Side, precise: Precise) -> Result<Checkout, Error> {
    let graph_of = |root: &Path, precise: Precise| {
        build_with(root, precise, &|_| {})
            .map_err(|err| Error(format!("cannot read {}: {err}", root.display())))
    };
    if matches!(side, Side::WorkTree) {
        return Ok(Checkout {
            root: repo.root().to_path_buf(),
            graph: graph_of(repo.root(), precise)?,
            _temporary: None,
        });
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let root = std::env::temp_dir().join(format!("mapdiff-side-{}-{unique}", std::process::id()));
    // Made before the export, so that a failed export is cleaned up too.
    let temporary = Temporary(root.clone());
    repo.export(side, &root)?;
    let graph = graph_of(&root, Precise::Off)?;
    Ok(Checkout {
        root,
        graph,
        _temporary: Some(temporary),
    })
}

/// Both sides of a comparison on disk, with their graphs.
pub struct Compared {
    pub head: Checkout,
    pub base: Checkout,
    /// The repository folder's name: what the root of a map is titled.
    name: String,
}

/// Puts both sides of a comparison on disk and builds their graphs.
pub fn compare(repo: &Repo, comparison: &Comparison, precise: Precise) -> Result<Compared, Error> {
    Ok(Compared {
        head: checkout(repo, comparison.head_side(), precise)?,
        base: checkout(repo, &comparison.base_side(), Precise::Off)?,
        name: repo
            .root()
            .canonicalize()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| String::from(".")),
    })
}

/// True for an entry of a change set that is drawn as a change of its own:
/// not a directory, and not a scope that is only listed because of what
/// changed inside it.
fn is_drawn(change: &Change) -> bool {
    change.kind != ScopeKind::Directory && !change.is_container_only()
}

/// True when the change is told from the side before it: the scope is no
/// longer where it was.
fn is_before(change: &Change) -> bool {
    matches!(change.state, ChangeState::Removed | ChangeState::MovedOut)
}

impl Compared {
    /// The map of the comparison: the map after the change, with what was
    /// only there before it kept as ghosts (see `union`), and the change
    /// set laid over it.
    pub fn map(&self, changes: &ChangeSet, level: Level, hidden: &HashSet<String>) -> MapData {
        let head_map = self.head.graph.map(&self.head.root, level, hidden);
        let base_map = self.base.graph.map(&self.base.root, level, hidden);
        let (mut map, image) = union_with_image(&base_map, &head_map, changes);
        // A side that was copied out to be read is in a folder with a
        // made-up name.
        for node in map.nodes.iter_mut().filter(|node| node.parent.is_none()) {
            node.label = self.name.clone();
        }
        let on_map: HashSet<&str> = map.nodes.iter().map(|node| node.id.as_str()).collect();
        let head_lifter = self.head.graph.lifter();
        let base_lifter = self.base.graph.lifter();

        // The box a scope is drawn in: by its ID after the change, or by
        // its ID before it for one that is gone.
        let box_after = |id: &str| -> Option<String> {
            head_lifter
                .at(id, level)
                .filter(|node| on_map.contains(node))
                .map(str::to_owned)
        };
        let box_before = |id: &str| -> Option<String> {
            let node = image.get(base_lifter.at(id, level)?)?;
            on_map.contains(node.as_str()).then(|| node.clone())
        };
        let box_of = |change: &Change| -> Option<String> {
            match is_before(change) {
                true => box_before(change.old_id.as_deref()?),
                false => box_after(change.new_id.as_deref()?),
            }
        };

        let mut overlay = Overlay::default();
        let drawn: Vec<usize> = (0..changes.changes.len())
            .filter(|position| is_drawn(&changes.changes[*position]))
            .collect();

        // A change to a test: by where its file is, or because its scope
        // is test code that was left out of the side it is on.
        fn known(graph: &Graph) -> HashSet<&str> {
            graph
                .tree
                .scopes
                .iter()
                .map(|scope| scope.id.as_str())
                .collect()
        }
        let (after_ids, before_ids) = (known(&self.head.graph), known(&self.base.graph));
        let is_test = |change: &Change| -> bool {
            let path = change.new_path.as_deref().or(change.old_path.as_deref());
            if path.is_some_and(mapdiff_extract::is_test_path) {
                return true;
            }
            match is_before(change) {
                true => change
                    .old_id
                    .as_deref()
                    .is_some_and(|id| !before_ids.contains(id)),
                false => change
                    .new_id
                    .as_deref()
                    .is_some_and(|id| !after_ids.contains(id)),
            }
        };
        let tests: HashSet<usize> = drawn
            .iter()
            .copied()
            .filter(|position| is_test(&changes.changes[*position]))
            .collect();
        overlay.tests = drawn
            .iter()
            .filter(|position| tests.contains(position))
            .map(|position| *position as u32)
            .collect();
        for position in &drawn {
            if let Some(node) = box_of(&changes.changes[*position]) {
                overlay
                    .changes
                    .entry(node)
                    .or_default()
                    .push(*position as u32);
            }
        }

        // Which change every scope inside a changed scope belongs to: the
        // innermost one. Larger scopes are filed first, so that a changed
        // method overrides its changed class.
        let mut owner_after: HashMap<String, usize> = HashMap::new();
        let mut owner_before: HashMap<String, usize> = HashMap::new();
        let mut by_size: Vec<(usize, HashSet<String>, bool)> = drawn
            .iter()
            .filter_map(|position| {
                let change = &changes.changes[*position];
                let before = is_before(change);
                let (graph, id) = match before {
                    true => (&self.base.graph, change.old_id.as_deref()?),
                    false => (&self.head.graph, change.new_id.as_deref()?),
                };
                Some((*position, graph.tree.subtree_ids(id), before))
            })
            .collect();
        by_size.sort_by_key(|(position, inside, _)| (std::cmp::Reverse(inside.len()), *position));
        for (position, inside, before) in by_size {
            let owners = if before {
                &mut owner_before
            } else {
                &mut owner_after
            };
            for id in inside {
                owners.insert(id, position);
            }
        }

        // Neighbours, and which change uses which.
        let mut near: BTreeSet<String> = BTreeSet::new();
        let mut uses: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let sides: [(&Graph, &HashMap<String, usize>, bool); 2] = [
            (&self.head.graph, &owner_after, false),
            (&self.base.graph, &owner_before, true),
        ];
        for (graph, owners, before) in sides {
            for reference in &graph.references.references {
                let from = owners.get(&reference.from);
                let to = owners.get(&reference.to);
                match (from, to) {
                    (Some(from), Some(to)) if from != to => {
                        uses.entry(*from).or_default().insert(*to);
                    }
                    (Some(_), None) | (None, Some(_)) => {
                        let other = match from {
                            Some(_) => &reference.to,
                            None => &reference.from,
                        };
                        let node = match before {
                            true => box_before(other),
                            false => box_after(other),
                        };
                        if let Some(node) = node {
                            near.insert(node);
                        }
                    }
                    _ => {}
                }
            }
        }
        // A box with a change of its own, or around one, is not a neighbour.
        let parent_of: HashMap<&str, Option<&str>> = map
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.parent.as_deref()))
            .collect();
        let mut around: HashSet<&str> = HashSet::new();
        for node in overlay.changes.keys() {
            let mut current = Some(node.as_str());
            while let Some(id) = current {
                if !around.insert(id) {
                    break;
                }
                current = parent_of.get(id).copied().flatten();
            }
        }
        // Nor is a box that holds a neighbour: an import at the top of a
        // file makes the file one, which says nothing once a function in
        // it is one too.
        let holds_one = |nodes: &BTreeSet<String>| -> HashSet<String> {
            let mut holders = HashSet::new();
            for node in nodes {
                let mut current = parent_of.get(node.as_str()).copied().flatten();
                while let Some(id) = current {
                    if !holders.insert(id.to_owned()) {
                        break;
                    }
                    current = parent_of.get(id).copied().flatten();
                }
            }
            holders
        };
        let holders = holds_one(&near);
        overlay.near = near
            .into_iter()
            .filter(|node| !around.contains(node.as_str()) && !holders.contains(node))
            .collect();

        // Unchanged users of a change they can see.
        let report = impact(
            changes,
            &self.head.graph,
            Some(&self.base.graph),
            ImpactOptions::default(),
        );
        for impacted in &report.impacted {
            let at = box_after(&impacted.id).or_else(|| box_before(&impacted.id));
            let Some(at) = at else { continue };
            let label = changes
                .changes
                .iter()
                .find(|change| change.id() == impacted.id)
                .map_or_else(|| impacted.id.clone(), |change| change.label.clone());
            for user in &impacted.affected {
                if user.also_changed || user.distance != 1 {
                    continue;
                }
                let Some(node) = box_after(&user.id) else {
                    continue;
                };
                if around.contains(node.as_str()) {
                    continue;
                }
                let causes = overlay.impacted.entry(node).or_default();
                if !causes
                    .iter()
                    .any(|cause| cause.at == at && cause.label == label)
                {
                    causes.push(Cause {
                        at: at.clone(),
                        label: label.clone(),
                        reasons: impacted
                            .reasons
                            .iter()
                            .map(|reason| reason.describe().to_owned())
                            .collect(),
                    });
                }
            }
        }

        let holders = holds_one(&overlay.impacted.keys().cloned().collect());
        overlay.impacted.retain(|node, _| !holders.contains(node));

        overlay.order = reading_order(changes, &drawn, &uses, &tests);
        map.overlay = Some(overlay);
        map
    }
}

/// The drawn changes in an order to read them in: a change comes after the
/// changes it uses, and the changes of one file stay together.
fn reading_order(
    changes: &ChangeSet,
    drawn: &[usize],
    uses: &BTreeMap<usize, BTreeSet<usize>>,
    tests: &HashSet<usize>,
) -> Vec<u32> {
    let path = |position: usize| {
        let change = &changes.changes[position];
        change
            .new_path
            .clone()
            .or_else(|| change.old_path.clone())
            .unwrap_or_default()
    };
    let line = |position: usize| {
        let change = &changes.changes[position];
        change
            .new_range
            .or(change.old_range)
            .map_or(0, |range| range.start.line)
    };
    // Code, then its tests, then prose: a test or a changelog is read once
    // the change it is about has been.
    let prose = |position: usize| match changes.changes[position].kind {
        ScopeKind::Document => 2,
        _ if tests.contains(&position) => 1,
        _ => 0,
    };
    // Start from the order of the files and of the lines in them, so that
    // nothing but the references decides where two changes differ from it.
    let mut starts: Vec<usize> = drawn.to_vec();
    starts.sort_by_key(|position| {
        (
            prose(*position),
            path(*position),
            line(*position),
            *position,
        )
    });

    // Depth first: a change is listed once everything it uses has been.
    let mut listed: Vec<usize> = Vec::new();
    let mut state: HashMap<usize, bool> = HashMap::new();
    for start in &starts {
        if state.contains_key(start) {
            continue;
        }
        let mut stack: Vec<(usize, Vec<usize>)> = vec![(*start, Vec::new())];
        state.insert(*start, false);
        let targets = |position: usize| -> Vec<usize> {
            let mut targets: Vec<usize> = uses
                .get(&position)
                .map(|set| set.iter().copied().collect())
                .unwrap_or_default();
            targets
                .sort_by_key(|target| std::cmp::Reverse((path(*target), line(*target), *target)));
            targets
        };
        stack[0].1 = targets(*start);
        while let Some((position, pending)) = stack.last_mut() {
            match pending.pop() {
                // Already listed, or being visited (a loop): leave it.
                Some(next) if state.contains_key(&next) => {}
                Some(next) => {
                    state.insert(next, false);
                    let below = targets(next);
                    stack.push((next, below));
                }
                None => {
                    state.insert(*position, true);
                    listed.push(*position);
                    stack.pop();
                }
            }
        }
    }

    // Keep each file's changes together, files in the order they first
    // come up.
    let mut file_rank: HashMap<String, usize> = HashMap::new();
    for position in &listed {
        let next = file_rank.len();
        file_rank.entry(path(*position)).or_insert(next);
    }
    // A test inside a source file (a Rust `mod tests`) is read with the
    // other tests, not with its file.
    let mut ranked: Vec<(u8, usize, usize, usize)> = listed
        .iter()
        .enumerate()
        .map(|(rank, position)| {
            (
                prose(*position),
                file_rank[&path(*position)],
                rank,
                *position,
            )
        })
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .map(|(_, _, _, position)| position as u32)
        .collect()
}

/// The map of a comparison. See `Compared::map`.
pub fn comparison_map(
    repo: &Repo,
    comparison: &Comparison,
    changes: &ChangeSet,
    level: Level,
    hidden: &HashSet<String>,
    precise: Precise,
) -> Result<MapData, Error> {
    Ok(compare(repo, comparison, precise)?.map(changes, level, hidden))
}
