//! The precise tier: finds which indexers apply to a repository, runs them,
//! keeps their output, and reads it back.
//!
//! An indexer is the language's own analysis (rust-analyzer, the TypeScript
//! compiler, pyright) writing what it knows as a SCIP index. Running one can
//! take from under a second to minutes, so its output is kept and reused
//! until a source file it covers changes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use mapdiff_core::{Language, ScopeKind};
use xxhash_rust::xxh3::Xxh3;

use crate::index::Index;
use crate::scip_import::ImportStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    RustAnalyzer,
    ScipTypescript,
    ScipPython,
}

impl Tool {
    pub fn program(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => "rust-analyzer",
            Tool::ScipTypescript => "scip-typescript",
            Tool::ScipPython => "scip-python",
        }
    }

    fn covers(self, language: Language) -> bool {
        matches!(
            (self, language),
            (Tool::RustAnalyzer, Language::Rust)
                | (
                    Tool::ScipTypescript,
                    Language::Typescript | Language::Javascript
                )
                | (Tool::ScipPython, Language::Python)
        )
    }

    /// How to get it, for the message shown when it is missing.
    pub fn install_hint(self) -> &'static str {
        match self {
            Tool::RustAnalyzer => {
                "brew install rust-analyzer, or rustup component add rust-analyzer"
            }
            Tool::ScipTypescript => "npm install -g @sourcegraph/scip-typescript",
            Tool::ScipPython => "npm install -g @sourcegraph/scip-python",
        }
    }
}

/// One run of one indexer: a directory and the tool to run in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub tool: Tool,
    /// Relative to the repository root; `.` for the root.
    pub directory: String,
    /// TypeScript only: there is no `tsconfig.json`, so the indexer must
    /// make one up (which is how plain JavaScript gets indexed).
    infer_config: bool,
}

/// How much of the precise tier to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Precise {
    /// Fast tier only.
    Off,
    /// Use indexes that were made earlier and still match the files.
    #[default]
    Cached,
    /// Also run the indexers for anything that has no matching index.
    Refresh,
}

/// What happened with one project.
#[derive(Debug, Clone, PartialEq)]
pub struct TierReport {
    pub tool: &'static str,
    pub directory: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The index was read.
    Used {
        stats: ImportStats,
        /// False when the indexer was run just now.
        from_cache: bool,
        /// How long the indexer took, when it ran.
        seconds: f32,
    },
    /// There is no index for the files as they are now. Not an error in
    /// `Cached` mode: it means nobody asked for one yet.
    NotIndexed,
    ToolMissing {
        install: &'static str,
    },
    Failed(String),
}

/// The indexer runs a repository needs: one per Cargo workspace, one per
/// TypeScript project, one for all Python.
pub fn discover(index: &Index<'_>) -> Vec<Project> {
    let has = |tool: Tool| {
        index
            .scopes
            .iter()
            .any(|scope| scope.kind == ScopeKind::File && tool.covers(scope.language))
    };
    // Directories holding a file with one of these names, outermost only:
    // a config inside another's directory belongs to that outer project.
    let outermost = |names: &[&str]| -> Vec<String> {
        let mut directories: Vec<&str> = index
            .paths
            .keys()
            .filter(|path| names.contains(&path.rsplit('/').next().unwrap_or(path)))
            .map(|path| Index::directory_of(path))
            .collect();
        directories.sort_by_key(|directory| (directory.matches('/').count(), *directory));
        let mut kept: Vec<String> = Vec::new();
        for directory in directories {
            let nested = kept.iter().any(|outer| {
                outer == "."
                    || directory == outer
                    || directory
                        .strip_prefix(outer.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            });
            if !nested {
                kept.push(directory.to_owned());
            }
        }
        kept
    };

    let mut projects = Vec::new();
    if has(Tool::RustAnalyzer) {
        for directory in outermost(&["Cargo.toml"]) {
            projects.push(Project {
                tool: Tool::RustAnalyzer,
                directory,
                infer_config: false,
            });
        }
    }
    if has(Tool::ScipTypescript) {
        let configured = outermost(&["tsconfig.json", "jsconfig.json"]);
        if configured.is_empty() {
            projects.push(Project {
                tool: Tool::ScipTypescript,
                directory: String::from("."),
                infer_config: true,
            });
        }
        for directory in configured {
            projects.push(Project {
                tool: Tool::ScipTypescript,
                directory,
                infer_config: false,
            });
        }
    }
    if has(Tool::ScipPython) {
        projects.push(Project {
            tool: Tool::ScipPython,
            directory: String::from("."),
            infer_config: false,
        });
    }
    projects
}

impl Project {
    /// Changes whenever a source file this project covers changes, so an
    /// index is reused exactly as long as it is still true.
    fn key(&self, index: &Index<'_>) -> String {
        let mut hasher = Xxh3::new();
        hasher.update(self.tool.program().as_bytes());
        hasher.update(self.directory.as_bytes());
        let mut files: Vec<(&str, &str)> = index
            .scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::File && self.tool.covers(scope.language))
            .filter(|scope| inside(&scope.path.file, &self.directory))
            .map(|scope| (scope.path.file.as_str(), scope.hashes.text.as_str()))
            .collect();
        files.sort();
        for (path, hash) in files {
            hasher.update(path.as_bytes());
            hasher.update(&[0x1f]);
            hasher.update(hash.as_bytes());
            hasher.update(&[0x1e]);
        }
        format!("{:016x}", hasher.digest())
    }

    fn cache_file(&self, root: &Path, index: &Index<'_>, cache: Option<&Path>) -> PathBuf {
        let name = format!(
            "{}-{}-{}.scip",
            self.tool.program(),
            self.directory.replace(['/', '.'], "_"),
            self.key(index)
        );
        cache_directory(root, cache).join(name)
    }

    /// The bytes of a matching index made earlier, when there is one.
    pub fn cached(&self, root: &Path, index: &Index<'_>, cache: Option<&Path>) -> Option<Vec<u8>> {
        std::fs::read(self.cache_file(root, index, cache)).ok()
    }

    /// Runs the indexer and returns the index it wrote, which is also kept
    /// for next time. Older indexes of the same project are removed.
    pub fn run(
        &self,
        root: &Path,
        index: &Index<'_>,
        cache: Option<&Path>,
    ) -> Result<(Vec<u8>, f32), Outcome> {
        let program = self.tool.program();
        if Command::new(program).arg("--version").output().is_err() {
            return Err(Outcome::ToolMissing {
                install: self.tool.install_hint(),
            });
        }
        let fail = |message: String| Outcome::Failed(message);

        let target = self.cache_file(root, index, cache);
        let directory = target.parent().unwrap_or(Path::new(".")).to_path_buf();
        std::fs::create_dir_all(&directory)
            .map_err(|err| fail(format!("cannot create {}: {err}", directory.display())))?;
        let unfinished = target.with_extension("scip.tmp");

        // Indexers want absolute paths: scip-python writes an empty index
        // when its working directory is given as a relative one.
        let absolute_root = root.canonicalize().map_err(|err| fail(err.to_string()))?;
        let project = if self.directory == "." {
            absolute_root.clone()
        } else {
            absolute_root.join(&self.directory)
        };
        let output_path = unfinished.to_string_lossy().into_owned();
        let project_path = project.to_string_lossy().into_owned();

        let mut command = Command::new(program);
        command.current_dir(&project);
        match self.tool {
            Tool::RustAnalyzer => {
                command.args(["scip", &project_path, "--output", &output_path]);
            }
            Tool::ScipTypescript => {
                command.args([
                    "index",
                    "--cwd",
                    &project_path,
                    "--output",
                    &output_path,
                    "--no-progress-bar",
                ]);
                if self.infer_config {
                    command.arg("--infer-tsconfig");
                }
            }
            Tool::ScipPython => {
                let name = absolute_root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| String::from("project"));
                command.args([
                    "index",
                    "--cwd",
                    &project_path,
                    "--output",
                    &output_path,
                    "--quiet",
                ]);
                // A fixed version keeps it from asking git or pip for one.
                command.args(["--project-name", &name, "--project-version", "0"]);
            }
        }

        // With `--infer-tsconfig` the indexer leaves a `tsconfig.json`
        // behind. It must not stay in the user's working tree.
        let inferred_config = project.join("tsconfig.json");
        let remove_config = self.infer_config && !inferred_config.exists();

        let started = Instant::now();
        let result = command.output();
        if remove_config {
            let _ = std::fs::remove_file(&inferred_config);
        }
        let output = result.map_err(|err| fail(format!("cannot run {program}: {err}")))?;
        let seconds = started.elapsed().as_secs_f32();

        if !output.status.success() {
            let _ = std::fs::remove_file(&unfinished);
            let text = String::from_utf8_lossy(&output.stderr);
            let last = text
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("no message");
            return Err(fail(format!("{program} failed: {}", last.trim())));
        }
        let bytes =
            std::fs::read(&unfinished).map_err(|_| fail(format!("{program} wrote no index")))?;

        // Replace earlier indexes of this project with the new one.
        let prefix = format!("{}-{}-", program, self.directory.replace(['/', '.'], "_"));
        if let Ok(entries) = std::fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) && name.ends_with(".scip") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        std::fs::rename(&unfinished, &target)
            .map_err(|err| fail(format!("cannot keep the index: {err}")))?;
        Ok((bytes, seconds))
    }
}

fn inside(path: &str, directory: &str) -> bool {
    directory == "."
        || path
            .strip_prefix(directory)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Where indexes for a repository are kept: outside the repository, in the
/// user's cache directory, under a name made from the repository's path.
/// A caller can name another base directory, and so can `MAPDIFF_CACHE_DIR`.
fn cache_directory(root: &Path, base: Option<&Path>) -> PathBuf {
    let base = match (base, std::env::var_os("MAPDIFF_CACHE_DIR")) {
        (Some(directory), _) => directory.to_path_buf(),
        (None, Some(directory)) => PathBuf::from(directory),
        (None, None) => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            if cfg!(target_os = "macos") {
                home.join("Library/Caches/mapdiff")
            } else {
                match std::env::var_os("XDG_CACHE_HOME") {
                    Some(cache) => PathBuf::from(cache).join("mapdiff"),
                    None => home.join(".cache/mapdiff"),
                }
            }
        }
    };
    let absolute = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = Xxh3::new();
    hasher.update(absolute.to_string_lossy().as_bytes());
    let name = absolute
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    base.join("index")
        .join(format!("{name}-{:016x}", hasher.digest()))
}
