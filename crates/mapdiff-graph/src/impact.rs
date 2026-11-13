//! The impact of a change: which scopes, not changed themselves, use
//! something that changed in a way a user of it can see.

use std::collections::{BTreeMap, HashMap, HashSet};

use mapdiff_core::{Change, ChangeKind, ChangeSet, ChangeState, Reference};

use crate::Graph;

/// Why a changed scope matters to the scopes that use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    Removed,
    Signature,
    Renamed,
    Moved,
    Access,
    /// Only reported when asked for: a body can change without its users
    /// having to.
    Body,
}

impl Reason {
    pub fn describe(self) -> &'static str {
        match self {
            Reason::Removed => "removed",
            Reason::Signature => "signature changed",
            Reason::Renamed => "renamed",
            Reason::Moved => "moved",
            Reason::Access => "access changed",
            Reason::Body => "body changed",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ImpactOptions {
    /// How many steps of use to follow. 1 lists direct users only.
    pub depth: u32,
    /// Also count a changed body as something users are affected by.
    pub bodies: bool,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        Self {
            depth: 1,
            bodies: false,
        }
    }
}

/// A scope that uses a changed scope, directly or through others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Affected {
    pub id: String,
    /// 1 for a direct user.
    pub distance: u32,
    /// True when this scope is itself part of the change. Its author has
    /// seen it; the ones to look at are those where this is false.
    pub also_changed: bool,
    /// For a direct user: where it uses the changed scope.
    pub references: Vec<Reference>,
}

/// One changed scope and what uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impacted {
    /// The changed scope: its ID after the change, or before it when it
    /// was removed.
    pub id: String,
    pub reasons: Vec<Reason>,
    pub affected: Vec<Affected>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImpactReport {
    /// Changed scopes that have at least one user, by ID.
    pub impacted: Vec<Impacted>,
    /// Scopes changed in a way users can see, with or without users.
    pub visible_changes: u32,
    /// Distinct scopes that use one of them and are not part of the change.
    pub unchanged_users: u32,
    /// True when a scope was removed but the graph before the change was
    /// not supplied, so its users could not be looked up.
    pub removed_not_checked: bool,
}

/// Works out the impact of a change set.
///
/// `head` is the graph after the change. `base` is the graph before it,
/// which is only needed to find the users of scopes that were removed.
pub fn impact(
    changes: &ChangeSet,
    head: &Graph,
    base: Option<&Graph>,
    options: ImpactOptions,
) -> ImpactReport {
    // Scopes that changed themselves, by their ID after the change.
    let changed_ids: HashSet<&str> = changes
        .changes
        .iter()
        .filter(|change| change.is_scope() && !change.is_container_only())
        .filter_map(|change| change.new_id.as_deref())
        .collect();
    // Where each scope that survived is now, for users found in `base`.
    let moved_to: HashMap<&str, &str> = changes
        .changes
        .iter()
        .filter_map(|change| Some((change.old_id.as_deref()?, change.new_id.as_deref()?)))
        .collect();
    let moved_away: HashMap<&str, &str> = changes
        .changes
        .iter()
        .filter(|change| change.state == ChangeState::MovedOut)
        .filter_map(|change| Some((change.old_id.as_deref()?, change.counterpart.as_deref()?)))
        .collect();

    let parent_of: HashMap<&str, Option<&str>> = head
        .tree
        .scopes
        .iter()
        .map(|scope| (scope.id.as_str(), scope.parent.as_deref()))
        .collect();
    // Changed itself, or inside something that was added or changed whole.
    let is_changed = |id: &str| {
        let mut current = Some(id);
        while let Some(scope) = current {
            if changed_ids.contains(scope) {
                return true;
            }
            current = parent_of.get(scope).copied().flatten();
        }
        false
    };

    let mut report = ImpactReport::default();
    let mut unchanged: HashSet<String> = HashSet::new();

    for change in &changes.changes {
        if !change.is_scope() {
            continue;
        }
        let reasons = reasons_for(change, options);
        if reasons.is_empty() {
            continue;
        }
        report.visible_changes += 1;

        let removed = change.state == ChangeState::Removed;
        let (graph, id) = match (removed, base) {
            (false, _) => (head, change.new_id.as_deref()),
            (true, Some(base)) => (base, change.old_id.as_deref()),
            (true, None) => {
                report.removed_not_checked = true;
                continue;
            }
        };
        let Some(id) = id else { continue };
        let inside = graph.tree.subtree_ids(id);
        if inside.is_empty() {
            continue;
        }

        // A user found before the change is named as it is after it.
        let now = |user: &str| -> Option<String> {
            if !removed {
                return Some(user.to_owned());
            }
            let current = moved_to
                .get(user)
                .or(moved_away.get(user))
                .copied()
                .unwrap_or(user);
            parent_of.contains_key(current).then(|| current.to_owned())
        };

        let mut direct: BTreeMap<String, Vec<Reference>> = BTreeMap::new();
        for reference in graph.references.incoming(&inside) {
            if let Some(user) = now(&reference.from) {
                direct.entry(user).or_default().push(reference.clone());
            }
        }
        let mut affected: Vec<Affected> = Vec::new();
        for (distance, user) in graph.references.callers(&inside, options.depth.max(1)) {
            let Some(user) = now(&user) else { continue };
            if affected.iter().any(|known| known.id == user) {
                continue;
            }
            let also_changed = is_changed(&user);
            if !also_changed {
                unchanged.insert(user.clone());
            }
            affected.push(Affected {
                references: if distance == 1 {
                    direct.remove(&user).unwrap_or_default()
                } else {
                    Vec::new()
                },
                id: user,
                distance,
                also_changed,
            });
        }
        if affected.is_empty() {
            continue;
        }
        affected.sort_by(|a, b| (a.distance, &a.id).cmp(&(b.distance, &b.id)));
        report.impacted.push(Impacted {
            id: id.to_owned(),
            reasons,
            affected,
        });
    }

    report.impacted.sort_by(|a, b| a.id.cmp(&b.id));
    report.unchanged_users = unchanged.len() as u32;
    report
}

/// The ways a change is visible to users of the scope.
fn reasons_for(change: &Change, options: ImpactOptions) -> Vec<Reason> {
    let mut reasons = Vec::new();
    match change.state {
        ChangeState::Removed => reasons.push(Reason::Removed),
        ChangeState::Renamed => reasons.push(Reason::Renamed),
        ChangeState::MovedIn => reasons.push(Reason::Moved),
        // The arriving end of a move stands for both; an added scope has no
        // users from before.
        ChangeState::MovedOut | ChangeState::Added => return reasons,
        ChangeState::Modified => {}
    }
    if change.state != ChangeState::Removed {
        if change.kinds.contains(&ChangeKind::Signature) && change.state != ChangeState::Renamed {
            reasons.push(Reason::Signature);
        }
        if change.kinds.contains(&ChangeKind::Access) {
            reasons.push(Reason::Access);
        }
        if options.bodies && change.kinds.contains(&ChangeKind::Body) {
            reasons.push(Reason::Body);
        }
    }
    reasons.sort();
    reasons
}
