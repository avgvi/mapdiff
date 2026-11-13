//! JavaScript and TypeScript: which file a module specifier names.
//!
//! Covers relative paths, `tsconfig.json` path aliases, and packages that
//! live in the repository. Anything in `node_modules` is outside the
//! repository and resolves to nothing.

use std::path::Path;

use mapdiff_core::{Modifier, ScopeKind};
use serde_json::Value;

use crate::index::Index;

const EXTENSIONS: [&str; 9] = [
    ".ts", ".tsx", ".d.ts", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs",
];

/// A package defined in the repository.
struct Package {
    name: String,
    directory: String,
    /// The file `import "name"` loads, relative to the directory.
    entry: Option<String>,
}

/// The `paths` and `baseUrl` of one `tsconfig.json` or `jsconfig.json`.
struct Aliases {
    /// The directory the config file is in.
    directory: String,
    /// Where `baseUrl` points, when it is set.
    base: Option<String>,
    /// Pattern and replacements, both possibly with one `*`.
    paths: Vec<(String, Vec<String>)>,
}

pub struct EcmaModules {
    packages: Vec<Package>,
    aliases: Vec<Aliases>,
}

impl EcmaModules {
    pub fn new(index: &Index<'_>, root: &Path) -> Self {
        let mut packages = Vec::new();
        let mut aliases = Vec::new();
        let mut paths: Vec<&str> = index.paths.keys().copied().collect();
        paths.sort();
        for path in paths {
            let name = path.rsplit('/').next().unwrap_or(path);
            if !matches!(name, "package.json" | "tsconfig.json" | "jsconfig.json") {
                continue;
            }
            let Some(json) = std::fs::read_to_string(root.join(path))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&strip_json_comments(&text)).ok())
            else {
                continue;
            };
            let directory = Index::directory_of(path).to_owned();
            if name == "package.json" {
                if let Some(package_name) = json.get("name").and_then(Value::as_str) {
                    packages.push(Package {
                        name: package_name.to_owned(),
                        directory,
                        entry: package_entry(&json),
                    });
                }
            } else {
                let options = json.get("compilerOptions");
                let base = options
                    .and_then(|options| options.get("baseUrl"))
                    .and_then(Value::as_str)
                    .and_then(|base| Index::join(&directory, base));
                let paths = options
                    .and_then(|options| options.get("paths"))
                    .and_then(Value::as_object)
                    .map(|paths| {
                        paths
                            .iter()
                            .map(|(pattern, targets)| {
                                let targets = targets
                                    .as_array()
                                    .map(|targets| {
                                        targets
                                            .iter()
                                            .filter_map(Value::as_str)
                                            .map(str::to_owned)
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                (pattern.clone(), targets)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                aliases.push(Aliases {
                    directory,
                    base,
                    paths,
                });
            }
        }
        // The nearest config wins, so try deeper directories first.
        aliases.sort_by_key(|config| std::cmp::Reverse(config.directory.len()));
        Self { packages, aliases }
    }

    /// The file a specifier names, seen from the importing file.
    pub fn resolve(&self, index: &Index<'_>, from: usize, specifier: &str) -> Option<usize> {
        let from_path = index.scopes[from].path.file.as_str();
        let directory = Index::directory_of(from_path);
        if specifier.starts_with('.') {
            return candidates(index, &Index::join(directory, specifier)?);
        }
        if specifier.starts_with('/') {
            return None;
        }

        for config in &self.aliases {
            if !inside_directory(from_path, &config.directory) {
                continue;
            }
            // `paths` targets are relative to `baseUrl`, or to the config.
            let anchor = config.base.as_deref().unwrap_or(&config.directory);
            for (pattern, targets) in &config.paths {
                let Some(captured) = match_pattern(pattern, specifier) else {
                    continue;
                };
                for target in targets {
                    let replaced = target.replacen('*', captured, 1);
                    if let Some(found) =
                        Index::join(anchor, &replaced).and_then(|path| candidates(index, &path))
                    {
                        return Some(found);
                    }
                }
            }
            if let Some(base) = &config.base
                && let Some(found) =
                    Index::join(base, specifier).and_then(|path| candidates(index, &path))
            {
                return Some(found);
            }
            break;
        }

        for package in &self.packages {
            if specifier == package.name {
                let entry = package.entry.as_deref().unwrap_or("index");
                return Index::join(&package.directory, entry)
                    .and_then(|path| candidates(index, &path));
            }
            if let Some(rest) = specifier
                .strip_prefix(package.name.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
            {
                return Index::join(&package.directory, rest)
                    .and_then(|path| candidates(index, &path));
            }
        }
        None
    }

    /// What `import x from` or a bare `require()` gives: the scope marked as
    /// the default export, the object assigned to `module.exports`, or the
    /// file when it has neither.
    pub fn default_export(&self, index: &Index<'_>, file: usize) -> usize {
        index.children[file]
            .iter()
            .copied()
            .find(|child| {
                let scope = &index.scopes[*child];
                scope.has(Modifier::DefaultExport)
                    || matches!(
                        scope.path.segments.last().map(String::as_str),
                        Some("{default}" | "{exports}")
                    )
            })
            .unwrap_or(file)
    }

    /// A named export that is not a top-level scope of its own: a method of
    /// the object assigned to `module.exports`.
    pub fn export_named(&self, index: &Index<'_>, file: usize, name: &str) -> Option<usize> {
        if name == "default" {
            let found = self.default_export(index, file);
            return (found != file).then_some(found);
        }
        index.children[file]
            .iter()
            .copied()
            .filter(|child| {
                index.scopes[*child]
                    .path
                    .segments
                    .last()
                    .map(String::as_str)
                    == Some("{exports}")
            })
            .find_map(|exports| index.child_named(exports, name))
    }
}

/// The file a path without (or with the wrong) extension names.
fn candidates(index: &Index<'_>, path: &str) -> Option<usize> {
    let file = |candidate: &str| {
        index
            .paths
            .get(candidate)
            .copied()
            .filter(|scope| index.kind(*scope) != ScopeKind::Directory)
    };
    if let Some(found) = file(path) {
        return Some(found);
    }
    // TypeScript sources are imported by the name of the file they compile to.
    for (written, sources) in [
        (".js", [".ts", ".tsx"]),
        (".mjs", [".mts", ".mts"]),
        (".cjs", [".cts", ".cts"]),
        (".jsx", [".tsx", ".tsx"]),
    ] {
        if let Some(stem) = path.strip_suffix(written) {
            for source in sources {
                if let Some(found) = file(&format!("{stem}{source}")) {
                    return Some(found);
                }
            }
        }
    }
    EXTENSIONS
        .iter()
        .find_map(|extension| file(&format!("{path}{extension}")))
        .or_else(|| {
            EXTENSIONS
                .iter()
                .find_map(|extension| file(&format!("{path}/index{extension}")))
        })
}

fn inside_directory(path: &str, directory: &str) -> bool {
    directory == "."
        || path
            .strip_prefix(directory)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Matches a `paths` pattern with at most one `*` and returns what the star
/// stands for. A pattern without a star must match exactly.
fn match_pattern<'a>(pattern: &str, specifier: &'a str) -> Option<&'a str> {
    match pattern.split_once('*') {
        Some((before, after)) => specifier.strip_prefix(before)?.strip_suffix(after),
        None => (pattern == specifier).then_some(""),
    }
}

/// The file a package's bare name loads: `exports["."]`, `module` or `main`.
fn package_entry(json: &Value) -> Option<String> {
    fn pick(value: &Value) -> Option<String> {
        match value {
            Value::String(path) => Some(path.clone()),
            Value::Object(conditions) => ["import", "require", "default"]
                .iter()
                .find_map(|condition| conditions.get(*condition).and_then(pick)),
            _ => None,
        }
    }
    let exports = json.get("exports");
    exports
        .and_then(|exports| exports.get("."))
        .and_then(pick)
        .or_else(|| exports.filter(|exports| exports.is_string()).and_then(pick))
        .or_else(|| {
            json.get("module")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| json.get("main").and_then(Value::as_str).map(str::to_owned))
}

/// Removes `//` and `/* */` comments and trailing commas, which config files
/// allow and JSON does not. Strings are left alone.
fn strip_json_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(character) = chars.next() {
        if in_string {
            out.push(character);
            match character {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (character, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(character);
            }
            ('/', Some('/')) => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut previous = ' ';
                for skipped in chars.by_ref() {
                    if previous == '*' && skipped == '/' {
                        break;
                    }
                    previous = skipped;
                }
            }
            (',', _) => {
                // Drop the comma when only whitespace separates it from a closer.
                let mut ahead = chars.clone();
                let next = ahead.find(|c| !c.is_whitespace());
                if !matches!(next, Some('}' | ']')) {
                    out.push(',');
                }
            }
            _ => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_files_with_comments_and_trailing_commas_parse() {
        let text = r#"{
            // a comment
            "compilerOptions": {
                "baseUrl": ".", /* inline */
                "paths": { "@/*": ["./src/*"], },
            },
            "url": "http://example.invalid/a,}",
        }"#;
        let json: Value = serde_json::from_str(&strip_json_comments(text)).unwrap();
        assert_eq!(json["compilerOptions"]["paths"]["@/*"][0], "./src/*");
        assert_eq!(json["url"], "http://example.invalid/a,}");
    }

    #[test]
    fn path_patterns_capture_the_star() {
        assert_eq!(match_pattern("@/*", "@/models"), Some("models"));
        assert_eq!(match_pattern("lib", "lib"), Some(""));
        assert_eq!(match_pattern("lib", "library"), None);
        assert_eq!(match_pattern("#*-impl", "#shape-impl"), Some("shape"));
    }
}
