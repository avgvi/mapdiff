//! Turns the facts collected from each file into references between scopes.
//!
//! Resolution uses names and imports only: no types. A path such as
//! `helpers.normalise` is resolved one name at a time, as far as it goes.
//! What cannot be resolved (a method called on a value, a name from the
//! standard library) is counted and left out.

use std::collections::HashMap;
use std::path::Path;

use mapdiff_core::{Language, RefKind, Reference, ScopeKind, Tier};

use crate::facts::{Binding, Facts, SELF, SELF_TYPE, SUPER, Seg, Target, Use};
use crate::index::Index;
use crate::{modules_ecma::EcmaModules, modules_python::PythonModules, modules_rust::RustModules};

/// How many imports, re-exports or base classes one lookup may follow. It
/// stops chains that loop back on themselves.
const MAX_HOPS: u32 = 8;

pub struct Resolver<'a> {
    pub index: &'a Index<'a>,
    /// Import bindings by the scope they are visible in.
    bindings: HashMap<usize, Vec<Binding>>,
    /// Base class paths by class scope.
    bases: HashMap<usize, Vec<Vec<Seg>>>,
    python: PythonModules,
    ecma: EcmaModules,
    rust: RustModules,
}

impl<'a> Resolver<'a> {
    pub fn new(index: &'a Index<'a>, root: &Path, facts: &HashMap<usize, Facts>) -> Self {
        let mut bindings: HashMap<usize, Vec<Binding>> = HashMap::new();
        let mut bases = HashMap::new();
        for (file, facts) in facts {
            for binding in &facts.bindings {
                bindings
                    .entry(container_of(index, *file, binding.statement))
                    .or_default()
                    .push(binding.clone());
            }
            for class in &facts.bases {
                let scope = index.innermost(*file, class.class);
                if let Some(scope) = index.enclosing_type(scope) {
                    bases.insert(scope, class.paths.clone());
                }
            }
        }
        Self {
            index,
            bindings,
            bases,
            python: PythonModules::new(index),
            ecma: EcmaModules::new(index, root),
            rust: RustModules::new(index, root),
        }
    }

    fn language(&self, scope: usize) -> Language {
        self.index.scopes[self.index.file_of[scope]].language
    }

    /// Every reference one file makes, and how many of its names could not
    /// be resolved.
    pub fn file_references(&self, file: usize, facts: &Facts) -> (Vec<Reference>, u32) {
        let mut references = Vec::new();
        let mut unresolved = 0;

        for used in &facts.uses {
            let from = self.index.innermost(file, used.segments[0].start);
            match self.resolve_use(from, used) {
                Some((target, segment)) => {
                    self.push(
                        &mut references,
                        from,
                        target,
                        used.kind,
                        &used.segments[segment],
                    );
                }
                None => unresolved += 1,
            }
        }

        // An import is itself a reference, from the statement to its target.
        for binding in &facts.bindings {
            let from = self.index.innermost(file, binding.statement);
            let container = container_of(self.index, file, binding.statement);
            if let Some(target) = self.resolve_binding(container, binding, 0) {
                let place = Seg {
                    name: String::new(),
                    start: binding.start,
                    end: binding.end,
                };
                self.push(&mut references, from, target, RefKind::Import, &place);
            }
        }

        // A Rust `mod name;` line refers to the file that holds the module.
        if self.language(file) == Language::Rust {
            for (declaration, target) in self.rust.declarations_in(self.index, file) {
                let scope = &self.index.scopes[declaration];
                let place = scope.name_range.unwrap_or(scope.range);
                references.push(Reference {
                    from: scope.id.clone(),
                    to: self.index.scopes[target].id.clone(),
                    kind: RefKind::Import,
                    tier: Tier::Fast,
                    range: place,
                });
            }
        }
        (references, unresolved)
    }

    fn push(&self, out: &mut Vec<Reference>, from: usize, to: usize, kind: RefKind, place: &Seg) {
        out.extend(self.reference(from, to, kind, Tier::Fast, place.start, place.end));
    }

    /// A reference between two scopes, or `None` when it would say nothing.
    /// Both tiers build their references here, so they follow one rule.
    pub fn reference(
        &self,
        from: usize,
        to: usize,
        kind: RefKind,
        tier: Tier,
        start: u32,
        end: u32,
    ) -> Option<Reference> {
        // A field declared inside a constructor (`self.x = f()`) is used by
        // the constructor, not by the field.
        let from = match (self.index.kind(from), self.index.parent[from]) {
            (ScopeKind::Field, Some(parent))
                if self.index.kind(parent) == ScopeKind::Constructor =>
            {
                parent
            }
            _ => from,
        };
        // A scope using itself, something it sits inside, or something
        // inside itself tells nothing about how scopes depend on each other.
        if self.index.is_inside(from, to) || self.index.is_inside(to, from) {
            return None;
        }
        Some(Reference {
            from: self.index.scopes[from].id.clone(),
            to: self.index.scopes[to].id.clone(),
            kind,
            tier,
            range: byte_range(start, end),
        })
    }

    /// Resolves a path as far as it goes. Returns the scope reached and the
    /// index of the last segment that was resolved.
    fn resolve_use(&self, from: usize, used: &Use) -> Option<(usize, usize)> {
        let first = used.segments[0].name.as_str();
        // Whether the name at a position is the one being called: only the
        // last name of a path is, and only when the path is a call.
        let last_is_call = |position: usize| {
            (position + 1 == used.segments.len()).then_some(used.kind == RefKind::Call)
        };
        let (mut target, mut reached) = match first {
            SELF | SELF_TYPE => {
                let own = self.index.enclosing_type(from)?;
                match used.segments.get(1) {
                    // `self.x` and `Self::x`: a member of the enclosing type.
                    Some(member) => (
                        self.lookup_member_as(own, &member.name, 0, last_is_call(1))?,
                        1,
                    ),
                    // `Self` alone names the type itself.
                    None => (self.rust.primary_type(self.index, own), 0),
                }
            }
            SUPER => {
                let own = self.index.enclosing_type(from)?;
                let base = self.base_classes(own, 0).into_iter().next()?;
                match used.segments.get(1) {
                    Some(member) => (
                        self.lookup_member_as(base, &member.name, 0, last_is_call(1))?,
                        1,
                    ),
                    None => (base, 0),
                }
            }
            name => (self.lookup_first(from, name, 0)?, 0),
        };

        while let Some(next) = used.segments.get(reached + 1) {
            match self.lookup_member_as(target, &next.name, 0, last_is_call(reached + 1)) {
                Some(found) => {
                    target = found;
                    reached += 1;
                }
                None => break,
            }
        }
        Some((target, reached))
    }

    /// Looks a bare name up from where it is used, outwards.
    pub fn lookup_first(&self, from: usize, name: &str, hops: u32) -> Option<usize> {
        if hops > MAX_HOPS {
            return None;
        }
        let language = self.language(from);
        if language == Language::Rust
            && let Some(found) = self.rust.keyword(self.index, from, name)
        {
            return Some(found);
        }

        let mut current = Some(from);
        while let Some(scope) = current {
            // The members of a type are not in scope inside it by bare name:
            // they need `self.`, `this.` or `Type::`.
            if !self.index.is_type_like(scope) {
                if let Some(found) = self.index.child_named(scope, name) {
                    return Some(self.follow_declaration(found));
                }
                if let Some(found) = self.lookup_binding(scope, name, false, hops) {
                    return Some(found);
                }
            }
            let kind = self.index.kind(scope);
            // A Rust module is a boundary: outer names need `super::`.
            let boundary = kind == ScopeKind::File
                || kind == ScopeKind::Document
                || (language == Language::Rust && kind == ScopeKind::Module);
            if boundary {
                break;
            }
            current = self.index.parent[scope];
        }

        match language {
            Language::Rust => self.rust.crate_named(name),
            _ => None,
        }
    }

    /// A Rust `mod name;` line stands for the file that holds the module.
    pub fn follow_declaration(&self, scope: usize) -> usize {
        self.rust.module_file(scope).unwrap_or(scope)
    }

    /// A name bound by an import that is visible in `container`.
    fn lookup_binding(
        &self,
        container: usize,
        name: &str,
        reexports: bool,
        hops: u32,
    ) -> Option<usize> {
        let bindings = self.bindings.get(&container)?;
        for binding in bindings {
            if binding.local.as_deref() == Some(name)
                && (reexports || !binding.reexport)
                && let Some(found) = self.resolve_binding(container, binding, hops + 1)
            {
                return Some(found);
            }
        }
        // `import *` and friends: the name may be any name of the target.
        for binding in bindings.iter().filter(|binding| binding.star) {
            if !reexports && binding.reexport {
                continue;
            }
            if let Some(module) = self.resolve_binding(container, binding, hops + 1)
                && let Some(found) = self.lookup_member(module, name, hops + 1)
            {
                return Some(found);
            }
        }
        None
    }

    /// The scope an import binding points at.
    fn resolve_binding(&self, container: usize, binding: &Binding, hops: u32) -> Option<usize> {
        if hops > MAX_HOPS {
            return None;
        }
        let file = self.index.file_of[container];
        match &binding.target {
            Target::Python {
                level,
                module,
                name,
            } => {
                let found = self.python.resolve(self.index, file, *level, module)?;
                match name {
                    Some(name) => self.lookup_member(found, name, hops + 1),
                    None => Some(found),
                }
            }
            Target::Ecma { specifier, name } => {
                let found = self.ecma.resolve(self.index, file, specifier)?;
                match name.as_deref() {
                    None => Some(found),
                    Some("default") => Some(self.ecma.default_export(self.index, found)),
                    Some(name) => self.lookup_member(found, name, hops + 1).or(Some(found)),
                }
            }
            Target::Rust { path } => {
                let mut segments = path.iter();
                let first = segments.next()?;
                // A `use` path starts from the module the statement is in.
                let mut target = self.lookup_use_root(container, first, hops + 1)?;
                for segment in segments {
                    target = self.lookup_member(target, segment, hops + 1)?;
                }
                Some(target)
            }
        }
    }

    /// The first name of a Rust `use` path. Unlike a name in code, it must
    /// not be looked up through the very `use` statements being resolved.
    fn lookup_use_root(&self, container: usize, name: &str, hops: u32) -> Option<usize> {
        if let Some(found) = self.rust.keyword(self.index, container, name) {
            return Some(found);
        }
        if let Some(found) = self.index.child_named(container, name) {
            return Some(self.follow_declaration(found));
        }
        self.rust
            .crate_named(name)
            .or_else(|| self.lookup_binding(container, name, false, hops + 1))
    }

    /// Looks a name up inside a scope that was already resolved: an export
    /// of a module, a member of a type, a method of an object literal.
    pub fn lookup_member(&self, target: usize, name: &str, hops: u32) -> Option<usize> {
        self.lookup_member_as(target, name, hops, None)
    }

    /// `lookup_member`, knowing whether the name is being called. That
    /// decides between a field and a method of the same name.
    fn lookup_member_as(
        &self,
        target: usize,
        name: &str,
        hops: u32,
        callable: Option<bool>,
    ) -> Option<usize> {
        if hops > MAX_HOPS {
            return None;
        }
        let target = self.follow_declaration(target);
        match self.index.kind(target) {
            ScopeKind::Directory => self.python.submodule(self.index, target, name),

            ScopeKind::File | ScopeKind::Document | ScopeKind::Module => {
                if self.language(target) == Language::Rust
                    && let Some(found) = self.rust.keyword_member(self.index, target, name)
                {
                    return Some(found);
                }
                if let Some(found) = self.index.child_named(target, name) {
                    return Some(self.follow_declaration(found));
                }
                if let Some(found) = self.lookup_binding(target, name, true, hops) {
                    return Some(found);
                }
                match self.language(target) {
                    Language::Python => self.python.submodule(self.index, target, name),
                    Language::Javascript | Language::Typescript => {
                        self.ecma.export_named(self.index, target, name)
                    }
                    _ => None,
                }
            }

            ScopeKind::Class | ScopeKind::Interface | ScopeKind::Enum | ScopeKind::Impl => {
                let mut members = self.index.members_named(target, name);
                // Rust keeps a type's members in separate `impl` blocks.
                members.extend(self.rust.type_members(self.index, target, name));
                // A field and a method can share a name. A call means the
                // method; anything else means the field.
                let preferred = callable.and_then(|wanted| {
                    members
                        .iter()
                        .copied()
                        .find(|member| self.index.is_callable(*member) == wanted)
                });
                if let Some(found) = preferred.or(members.first().copied()) {
                    return Some(found);
                }
                self.base_classes(target, hops)
                    .into_iter()
                    .find_map(|base| self.lookup_member_as(base, name, hops + 1, callable))
            }

            // An object literal bound to a name: its methods.
            ScopeKind::Constant => self.index.child_named(target, name),
            _ => None,
        }
    }

    fn base_classes(&self, class: usize, hops: u32) -> Vec<usize> {
        let Some(paths) = self.bases.get(&class) else {
            return Vec::new();
        };
        paths
            .iter()
            .filter_map(|path| {
                let mut target = self.lookup_first(class, &path.first()?.name, hops + 1)?;
                for segment in &path[1..] {
                    target = self.lookup_member(target, &segment.name, hops + 1)?;
                }
                (target != class && self.index.is_type_like(target)).then_some(target)
            })
            .collect()
    }
}

/// The scope an import statement's names are visible in: the scope around
/// the statement, which for a module-level import is the file or module.
fn container_of(index: &Index<'_>, file: usize, statement: u32) -> usize {
    let scope = index.innermost(file, statement);
    match index.kind(scope) {
        ScopeKind::Import | ScopeKind::Block => index.parent[scope].unwrap_or(file),
        _ => scope,
    }
}

/// A range that only knows its bytes. Lines and columns are filled in once
/// per file, after resolution.
fn byte_range(start: u32, end: u32) -> mapdiff_core::Range {
    mapdiff_core::Range {
        start_byte: start,
        end_byte: end,
        ..mapdiff_core::Range::default()
    }
}
