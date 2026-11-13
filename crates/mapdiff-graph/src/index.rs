//! Fast lookups over a scope tree: by ID, by path, by position, by name.

use std::collections::HashMap;

use mapdiff_core::{Scope, ScopeKind, ScopeTree};

pub struct Index<'a> {
    pub scopes: &'a [Scope],
    pub parent: Vec<Option<usize>>,
    pub children: Vec<Vec<usize>>,
    /// For each scope, the file it is in. Directories map to themselves.
    pub file_of: Vec<usize>,
    /// File and directory paths to their scope.
    pub paths: HashMap<&'a str, usize>,
}

impl<'a> Index<'a> {
    pub fn new(tree: &'a ScopeTree) -> Self {
        let scopes = tree.scopes.as_slice();
        let by_id: HashMap<&str, usize> = scopes
            .iter()
            .enumerate()
            .map(|(position, scope)| (scope.id.as_str(), position))
            .collect();

        let mut parent = vec![None; scopes.len()];
        let mut children = vec![Vec::new(); scopes.len()];
        let mut file_of: Vec<usize> = (0..scopes.len()).collect();
        let mut paths = HashMap::new();
        for (position, scope) in scopes.iter().enumerate() {
            parent[position] = scope
                .parent
                .as_deref()
                .and_then(|id| by_id.get(id).copied());
            children[position] = scope
                .children
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).copied())
                .collect();
            if scope.path.segments.is_empty() {
                paths.insert(scope.path.file.as_str(), position);
            } else if let Some(file) = by_id.get(scope.path.file.as_str()) {
                file_of[position] = *file;
            }
        }
        Self {
            scopes,
            parent,
            children,
            file_of,
            paths,
        }
    }

    pub fn kind(&self, scope: usize) -> ScopeKind {
        self.scopes[scope].kind
    }

    /// The innermost scope of a file that contains a byte.
    pub fn innermost(&self, file: usize, byte: u32) -> usize {
        let mut scope = file;
        loop {
            let children = &self.children[scope];
            let after =
                children.partition_point(|child| self.scopes[*child].range.start_byte <= byte);
            let inside = after
                .checked_sub(1)
                .map(|position| children[position])
                .filter(|child| byte < self.scopes[*child].range.end_byte);
            match inside {
                Some(child) => scope = child,
                None => return scope,
            }
        }
    }

    /// The first child with this name that is something one can refer to.
    pub fn child_named(&self, scope: usize, name: &str) -> Option<usize> {
        self.children[scope].iter().copied().find(|child| {
            let child = &self.scopes[*child];
            child.name.as_deref() == Some(name)
                && !matches!(child.kind, ScopeKind::Import | ScopeKind::Block)
        })
    }

    /// Every member of a type with this name: direct children, and fields
    /// declared inside one of its constructors (`self.x = ...`,
    /// `constructor(public x)`). There can be several: Rust lets a field and
    /// a method share a name.
    pub fn members_named(&self, scope: usize, name: &str) -> Vec<usize> {
        let named = |parent: usize| {
            self.children[parent].iter().copied().filter(move |child| {
                let child = &self.scopes[*child];
                child.name.as_deref() == Some(name)
                    && !matches!(child.kind, ScopeKind::Import | ScopeKind::Block)
            })
        };
        let mut found: Vec<usize> = named(scope).collect();
        for constructor in &self.children[scope] {
            if self.kind(*constructor) == ScopeKind::Constructor {
                found.extend(named(*constructor));
            }
        }
        found
    }

    /// True for scopes that are called rather than read.
    pub fn is_callable(&self, scope: usize) -> bool {
        matches!(
            self.kind(scope),
            ScopeKind::Function | ScopeKind::Method | ScopeKind::Constructor
        )
    }

    pub fn is_type_like(&self, scope: usize) -> bool {
        matches!(
            self.kind(scope),
            ScopeKind::Class | ScopeKind::Interface | ScopeKind::Enum | ScopeKind::Impl
        )
    }

    /// The nearest enclosing class, interface, enum or impl, or the scope
    /// itself when it is one.
    pub fn enclosing_type(&self, scope: usize) -> Option<usize> {
        let mut current = Some(scope);
        while let Some(scope) = current {
            if self.is_type_like(scope) {
                return Some(scope);
            }
            current = self.parent[scope];
        }
        None
    }

    /// True when `scope` is `ancestor` or lies inside it.
    pub fn is_inside(&self, scope: usize, ancestor: usize) -> bool {
        let mut current = Some(scope);
        while let Some(scope) = current {
            if scope == ancestor {
                return true;
            }
            current = self.parent[scope];
        }
        false
    }

    /// The directory part of a file or directory path: `a/b/c.py` -> `a/b`.
    /// The repository root is `.`.
    pub fn directory_of(path: &str) -> &str {
        match path.rsplit_once('/') {
            Some((directory, _)) => directory,
            None => ".",
        }
    }

    /// Joins a directory and a relative path, resolving `.` and `..`.
    /// `None` when the result would leave the repository.
    pub fn join(directory: &str, relative: &str) -> Option<String> {
        let mut parts: Vec<&str> = if directory == "." {
            Vec::new()
        } else {
            directory.split('/').collect()
        };
        for part in relative.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                other => parts.push(other),
            }
        }
        Some(if parts.is_empty() {
            String::from(".")
        } else {
            parts.join("/")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_join_and_stay_inside_the_repository() {
        assert_eq!(
            Index::join("src/app", "./util").as_deref(),
            Some("src/app/util")
        );
        assert_eq!(
            Index::join("src/app", "../lib/x").as_deref(),
            Some("src/lib/x")
        );
        assert_eq!(Index::join(".", "pkg/mod").as_deref(), Some("pkg/mod"));
        assert_eq!(Index::join("src", "..").as_deref(), Some("."));
        assert_eq!(Index::join("src", "../../outside"), None);
        assert_eq!(Index::directory_of("a/b/c.py"), "a/b");
        assert_eq!(Index::directory_of("c.py"), ".");
    }
}
