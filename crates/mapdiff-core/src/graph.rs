//! The reference graph: which scope refers to which.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Range, SCHEMA_VERSION};

/// How one scope uses another, read off the syntax around the referring name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// The name is called, or constructed with `new`.
    Call,
    /// The name is used as a type.
    TypeUse,
    /// An import statement brings the name in.
    Import,
    /// A class extends the name.
    Inherit,
    /// A type implements the name: `impl Trait for`, `implements`.
    Implement,
    /// A macro is invoked.
    Macro,
    /// Any other use: read, passed along, a field accessed.
    Access,
}

/// How a reference was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Resolved from names and imports alone. Works on any checkout, but
    /// misses what needs type information, such as a method called on a value.
    Fast,
    /// Reported by a language's own indexer.
    Precise,
}

/// One use of a scope by another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Reference {
    /// The innermost scope that contains the referring name.
    pub from: String,
    /// The scope the name refers to.
    pub to: String,
    pub kind: RefKind,
    pub tier: Tier,
    /// Where the referring name is, in the file `from` is in.
    pub range: Range,
}

/// How a wire relates to the layout it is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireKind {
    /// At least one reference goes straight from one end to the other.
    Direct,
    /// A direct wire that closes a loop. A layout flows the other way and
    /// draws this one against the flow.
    Cycle,
    /// No reference joins the two ends themselves: the path between them
    /// runs through nodes that are hidden.
    Bridge,
}

/// Every reference between two nodes of a view, drawn as one line.
///
/// A view shows the tree down to some level. References between scopes
/// below that level are lifted to the nodes that are shown: at file level,
/// a call between two functions becomes part of a wire between their files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wire {
    pub from: String,
    pub to: String,
    pub kind: WireKind,
    /// How many references the wire stands for. For a bridge, how many
    /// references leave `from` towards the hidden nodes.
    pub count: u32,
    /// How many of them came from the precise tier.
    pub precise: u32,
    pub kinds: BTreeMap<RefKind, u32>,
    /// A few of the references, to show what the wire is made of.
    pub samples: Vec<Reference>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStats {
    /// Source files that were read for references.
    pub files: u32,
    pub references: u32,
    /// Names that looked like references but could not be resolved to a
    /// scope in the repository. Most are standard-library or third-party
    /// names; the rest are what the fast tier cannot see.
    pub unresolved: u32,
    pub by_kind: BTreeMap<RefKind, u32>,
}

/// Every reference found in a repository, sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceGraph {
    pub schema_version: u32,
    pub stats: GraphStats,
    pub references: Vec<Reference>,
}

impl ReferenceGraph {
    pub fn new(mut references: Vec<Reference>, files: u32, unresolved: u32) -> Self {
        references.sort();
        references.dedup();
        let mut by_kind = BTreeMap::new();
        for reference in &references {
            *by_kind.entry(reference.kind).or_insert(0) += 1;
        }
        Self {
            schema_version: SCHEMA_VERSION,
            stats: GraphStats {
                files,
                references: references.len() as u32,
                unresolved,
                by_kind,
            },
            references,
        }
    }

    /// References that point into a set of scopes from outside it. The set
    /// is usually one scope and everything inside it, from
    /// `ScopeTree::subtree_ids`.
    pub fn incoming<'a>(
        &'a self,
        inside: &'a HashSet<String>,
    ) -> impl Iterator<Item = &'a Reference> {
        self.references.iter().filter(move |reference| {
            inside.contains(&reference.to) && !inside.contains(&reference.from)
        })
    }

    /// References made from inside a set of scopes to something outside it.
    pub fn outgoing<'a>(
        &'a self,
        inside: &'a HashSet<String>,
    ) -> impl Iterator<Item = &'a Reference> {
        self.references.iter().filter(move |reference| {
            inside.contains(&reference.from) && !inside.contains(&reference.to)
        })
    }

    /// Scopes that use the set, level by level: direct users at depth 1,
    /// their users at depth 2, and so on. Each scope appears once, at the
    /// depth it was first reached.
    pub fn callers(&self, inside: &HashSet<String>, depth: u32) -> Vec<(u32, String)> {
        self.walk(inside, depth, true)
    }

    /// Scopes that the set uses, level by level.
    pub fn dependencies(&self, inside: &HashSet<String>, depth: u32) -> Vec<(u32, String)> {
        self.walk(inside, depth, false)
    }

    fn walk(&self, inside: &HashSet<String>, depth: u32, incoming: bool) -> Vec<(u32, String)> {
        // Index once, so a deep walk does not rescan every reference per step.
        let mut next: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for reference in &self.references {
            let (key, value) = if incoming {
                (reference.to.as_str(), reference.from.as_str())
            } else {
                (reference.from.as_str(), reference.to.as_str())
            };
            next.entry(key).or_default().insert(value);
        }

        let mut seen: BTreeSet<String> = inside.iter().cloned().collect();
        let mut out = Vec::new();
        let mut frontier: BTreeSet<String> = if incoming {
            self.incoming(inside)
                .map(|reference| reference.from.clone())
                .collect()
        } else {
            self.outgoing(inside)
                .map(|reference| reference.to.clone())
                .collect()
        };
        for level in 1..=depth {
            frontier.retain(|scope| !seen.contains(scope));
            if frontier.is_empty() {
                break;
            }
            let mut following = BTreeSet::new();
            for scope in &frontier {
                seen.insert(scope.clone());
                out.push((level, scope.clone()));
                for other in next.get(scope.as_str()).into_iter().flatten() {
                    following.insert((*other).to_owned());
                }
            }
            frontier = following;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(from: &str, to: &str) -> Reference {
        Reference {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: RefKind::Call,
            tier: Tier::Fast,
            range: Range::default(),
        }
    }

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn callers_are_listed_by_depth_and_once() {
        let graph = ReferenceGraph::new(
            vec![
                reference("m.rs::a()", "m.rs::b()"),
                reference("m.rs::b()", "m.rs::c()"),
                reference("m.rs::x()", "m.rs::c()"),
                // A cycle must not loop forever.
                reference("m.rs::c()", "m.rs::a()"),
            ],
            1,
            0,
        );
        assert_eq!(
            graph.callers(&set(&["m.rs::c()"]), 3),
            [
                (1, String::from("m.rs::b()")),
                (1, String::from("m.rs::x()")),
                (2, String::from("m.rs::a()")),
            ]
        );
        assert_eq!(
            graph.dependencies(&set(&["m.rs::a()"]), 1),
            [(1, String::from("m.rs::b()"))]
        );
    }

    #[test]
    fn a_container_collects_what_its_members_use() {
        let graph = ReferenceGraph::new(
            vec![
                reference("m.rs::Point::new()", "m.rs::helper()"),
                reference("m.rs::Point::new()", "m.rs::Point::origin()"),
                reference("m.rs::main()", "m.rs::Point::new()"),
            ],
            1,
            0,
        );
        // What `Point` uses outside itself, and who uses anything in it.
        let point = set(&["m.rs::Point", "m.rs::Point::new()", "m.rs::Point::origin()"]);
        assert_eq!(
            graph.dependencies(&point, 1),
            [(1, String::from("m.rs::helper()"))]
        );
        assert_eq!(
            graph.callers(&point, 1),
            [(1, String::from("m.rs::main()"))]
        );
    }
}
