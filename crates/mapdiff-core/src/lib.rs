//! Shared data model for mapdiff: the parts of it that every other crate
//! needs.

use serde::{Deserialize, Serialize};

mod changes;
mod graph;
mod model;

pub use changes::{
    Change, ChangeKind, ChangeSet, ChangeState, ChangeStats, LineCount, Revision, change_kind_name,
    state_name,
};

pub use graph::{GraphStats, RefKind, Reference, ReferenceGraph, Tier, Wire, WireKind};
pub use model::{
    Hashes, Language, Modifier, Position, Range, Scope, ScopePath, ScopeTree, access_name,
    kind_name, modifier_name,
};

/// Version of the JSON the tools emit. Bump it on any breaking change to the
/// serialised model.
pub const SCHEMA_VERSION: u32 = 1;

/// The shared kind of a scope, the same across all languages.
///
/// A scope also carries the word its own language uses (`lang_kind`, for
/// example `trait` or `struct`), which the UI shows. `ScopeKind` is what
/// filters, colours and layout rules key on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Directory,
    File,
    /// A non-code file, or a code file that could not be parsed.
    Document,
    Module,
    Class,
    Interface,
    Enum,
    Variant,
    TypeAlias,
    Impl,
    Function,
    Method,
    Constructor,
    Field,
    Constant,
    Macro,
    Import,
    /// A run of statements with no name of its own.
    Block,
}

impl ScopeKind {
    /// Every kind, in the order the spec lists them.
    pub const ALL: [ScopeKind; 18] = [
        ScopeKind::Directory,
        ScopeKind::File,
        ScopeKind::Document,
        ScopeKind::Module,
        ScopeKind::Class,
        ScopeKind::Interface,
        ScopeKind::Enum,
        ScopeKind::Variant,
        ScopeKind::TypeAlias,
        ScopeKind::Impl,
        ScopeKind::Function,
        ScopeKind::Method,
        ScopeKind::Constructor,
        ScopeKind::Field,
        ScopeKind::Constant,
        ScopeKind::Macro,
        ScopeKind::Import,
        ScopeKind::Block,
    ];

    /// True for kinds that hold other scopes rather than code of their own.
    pub fn is_container(self) -> bool {
        matches!(
            self,
            ScopeKind::Directory
                | ScopeKind::File
                | ScopeKind::Module
                | ScopeKind::Class
                | ScopeKind::Interface
                | ScopeKind::Enum
                | ScopeKind::Impl
        )
    }
}

/// How far a scope is visible, reduced to three levels that every supported
/// language can express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Visible only inside the enclosing scope or file.
    Private,
    /// Visible beyond its own scope but not to everyone: `pub(crate)`,
    /// `protected`, a Python name left out of `__all__`.
    Restricted,
    /// Part of the public surface.
    Public,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_kind_once() {
        let mut kinds = ScopeKind::ALL.to_vec();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), ScopeKind::ALL.len());
    }

    /// The serialised names are part of the public JSON contract. This
    /// snapshot fails when one is added, removed or renamed.
    #[test]
    fn kind_names_are_stable() {
        let rows: Vec<_> = ScopeKind::ALL
            .iter()
            .map(|kind| {
                serde_json::json!({
                    "kind": kind,
                    "container": kind.is_container(),
                })
            })
            .collect();
        insta::assert_json_snapshot!(serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kinds": rows,
            "access": [Access::Private, Access::Restricted, Access::Public],
        }));
    }
}
