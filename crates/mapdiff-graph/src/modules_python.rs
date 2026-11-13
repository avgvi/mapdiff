//! Python: which file an `import` names.

use std::collections::HashMap;

use mapdiff_core::{Language, ScopeKind};

use crate::index::Index;

pub struct PythonModules {
    /// Dotted module names to the file (or, for a package without an
    /// `__init__.py`, the directory) that holds them.
    modules: HashMap<String, usize>,
}

impl PythonModules {
    pub fn new(index: &Index<'_>) -> Self {
        let mut files: Vec<(&str, usize)> = index
            .paths
            .iter()
            .filter(|(path, scope)| {
                path.ends_with(".py") && index.scopes[**scope].language == Language::Python
            })
            .map(|(path, scope)| (*path, *scope))
            .collect();
        files.sort();

        let mut modules = HashMap::new();
        let mut loose = Vec::new();
        for (path, scope) in files {
            let mut parts: Vec<&str> = path.trim_end_matches(".py").split('/').collect();
            // A package is a directory with an `__init__.py`. The module's
            // name starts at the outermost package directory.
            let mut first = parts.len() - 1;
            while first > 0 {
                let directory = parts[..first].join("/");
                if index
                    .paths
                    .contains_key(format!("{directory}/__init__.py").as_str())
                {
                    first -= 1;
                } else {
                    break;
                }
            }
            if parts.last() == Some(&"__init__") {
                parts.pop();
            }
            if first >= parts.len() {
                continue;
            }
            modules.entry(parts[first..].join(".")).or_insert(scope);
            // Without packages we cannot tell where the import root is, so a
            // loose file answers to every tail of its path.
            for start in 0..parts.len() {
                if start != first {
                    loose.push((parts[start..].join("."), scope));
                }
            }
        }
        for (name, scope) in loose {
            modules.entry(name).or_insert(scope);
        }
        Self { modules }
    }

    /// The file or package an import names. `level` counts the leading dots
    /// of a relative import.
    pub fn resolve(
        &self,
        index: &Index<'_>,
        from: usize,
        level: u32,
        module: &[String],
    ) -> Option<usize> {
        let from_path = index.scopes[from].path.file.as_str();
        let mut directory = Index::directory_of(from_path).to_owned();
        if level > 0 {
            for _ in 1..level {
                directory = Index::join(&directory, "..")?;
            }
            let path = Index::join(&directory, &module.join("/"))?;
            return file_or_package(index, &path);
        }
        self.modules
            .get(module.join(".").as_str())
            .copied()
            // A script importing the file next to it.
            .or_else(|| file_or_package(index, &Index::join(&directory, &module.join("/"))?))
    }

    /// A module inside a package: `pkg.name` when `pkg` was resolved to its
    /// `__init__.py` or its directory.
    pub fn submodule(&self, index: &Index<'_>, package: usize, name: &str) -> Option<usize> {
        let scope = &index.scopes[package];
        let path = scope.path.file.as_str();
        let directory = match scope.kind {
            ScopeKind::Directory => path,
            _ if path.ends_with("__init__.py") => Index::directory_of(path),
            _ => return None,
        };
        file_or_package(index, &Index::join(directory, name)?)
    }
}

fn file_or_package(index: &Index<'_>, path: &str) -> Option<usize> {
    let find = |candidate: String| index.paths.get(candidate.as_str()).copied();
    find(format!("{path}.py"))
        .or_else(|| find(format!("{path}/__init__.py")))
        .or_else(|| {
            find(path.to_owned()).filter(|scope| index.kind(*scope) == ScopeKind::Directory)
        })
}
