//! The `mapdiff` command.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use mapdiff_core::{Access, Change, ChangeKind, ChangeSet, ChangeState, ScopeTree};
use mapdiff_gitdiff::{Comparison, DiffTag, Options};

#[derive(Parser)]
#[command(
    name = "mapdiff",
    version,
    about = "Review a change on a map of your code.",
    long_about = "Review a change on a map of your code.\n\n\
        With no command, mapdiff opens the map of the repository you are in, in your browser:\n\n  \
        mapdiff                 the map, with your uncommitted changes drawn on it\n  \
        mapdiff main...HEAD     what this branch changed\n  \
        mapdiff --commit HEAD   one commit\n  \
        mapdiff --pr 123        a GitHub pull request",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    open: OpenArgs,
}

/// What to open in the browser, and how.
#[derive(clap::Args)]
struct OpenArgs {
    /// A change to draw on the map: `REV` (against your files as they are),
    /// `A..B`, or `A...B` (from where B branched off A).
    #[arg(value_name = "CHANGE")]
    what: Option<String>,
    /// One commit, against the commit before it.
    #[arg(long, value_name = "REV", conflicts_with_all = ["what", "staged", "pr"])]
    commit: Option<String>,
    /// What is staged, against the last commit.
    #[arg(long, conflicts_with_all = ["what", "pr"])]
    staged: bool,
    /// A GitHub pull request. It is fetched from `origin` first, into refs
    /// under `refs/mapdiff/pr/`.
    #[arg(long, value_name = "NUMBER", conflicts_with = "what")]
    pr: Option<u32>,
    /// A directory inside the repository to open.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// The port to listen on. The next free one is used when it is taken.
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// Print the address without opening the browser.
    #[arg(long)]
    no_open: bool,
    /// A built copy of the page to serve instead of the one inside mapdiff.
    #[arg(long, value_name = "DIR", hide = true)]
    ui_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the scope tree of a file or directory.
    Scopes {
        /// A source file, or a directory to walk. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Print the full model as JSON instead of the indented tree.
        #[arg(long)]
        json: bool,
    },
    /// Print the source text of one scope.
    Source {
        /// A scope ID as printed by `mapdiff scopes --json`, for example
        /// `src/lib.rs::total_area()`.
        id: String,
        /// The directory that scope IDs are relative to.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Show what changed between two versions, as changed scopes.
    ///
    /// With no argument: the last commit against the working tree.
    Diff(DiffArgs),
    /// Show what uses a scope and what it uses.
    ///
    /// References are found from names and imports, without type
    /// information: a method called on a value is not seen.
    Refs {
        /// A scope, file or directory ID, for example `src/lib.rs::main()`.
        id: String,
        /// Only list what uses the scope.
        #[arg(long, conflicts_with = "deps")]
        callers: bool,
        /// Only list what the scope uses.
        #[arg(long)]
        deps: bool,
        /// How many steps to follow. 1 lists direct references with where
        /// they are; more lists scopes by distance.
        #[arg(long, default_value_t = 1)]
        depth: u32,
        /// Print the references as JSON.
        #[arg(long)]
        json: bool,
        /// Use names and imports only, ignoring any index made by `mapdiff index`.
        #[arg(long)]
        fast: bool,
        /// The directory that scope IDs are relative to.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Print a summary of the reference graph of a directory.
    Graph {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Print every reference as JSON instead of the summary.
        #[arg(long)]
        json: bool,
        /// Use names and imports only, ignoring any index made by `mapdiff index`.
        #[arg(long)]
        fast: bool,
    },
    /// Show the wires among the children of a directory, file or type.
    ///
    /// References between scopes are lifted to the level asked for: at
    /// `package`, a call between two functions is part of a wire between
    /// their files.
    Wires {
        /// The container to look inside. Defaults to the whole repository.
        #[arg(default_value = ".")]
        id: String,
        /// How far down the tree to show.
        #[arg(long, value_enum, default_value_t = LevelArg::Package)]
        level: LevelArg,
        /// Leave these scopes out. Paths that ran through them become bridges.
        #[arg(long, value_name = "ID")]
        hide: Vec<String>,
        /// Use names and imports only, ignoring any index made by `mapdiff index`.
        #[arg(long)]
        fast: bool,
        /// The directory that scope IDs are relative to.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Print what a map draws at a level: the boxes, and the wires among
    /// the children of each.
    ///
    /// With `--compare`, `--commit` or `--staged`, the map of the change:
    /// what is there after it, plus what was only there before it.
    Map(MapArgs),
    /// Show which unchanged scopes use something a change altered.
    ///
    /// Counts scopes that were removed, renamed, moved, or had their
    /// signature or access changed. With no argument: the last commit
    /// against the working tree.
    Impact(ImpactArgs),
    /// Run each language's own indexer, for references that need types.
    ///
    /// Uses rust-analyzer, scip-typescript and scip-python when they are
    /// installed. Their output is kept outside the repository and reused
    /// until a source file changes. This can take minutes on a large
    /// project; the fast references work without it.
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Open the map in the browser. The same as running mapdiff with no
    /// command.
    ///
    /// Runs until Ctrl-C.
    Serve(OpenArgs),
    /// Print the raw syntax tree of one file. A development aid.
    #[command(hide = true)]
    DebugTree { file: PathBuf },
    /// Print a summary of a SCIP index file. A development aid.
    #[command(hide = true)]
    DebugScip {
        file: PathBuf,
        /// List the occurrences of symbols containing this text.
        symbol: Option<String>,
    },
}

#[derive(clap::Args)]
struct DiffArgs {
    /// What to compare: `REV` (against the working tree), `A..B`, or
    /// `A...B` (from where B branched off A).
    spec: Option<String>,
    /// Compare against what is staged instead of the working tree.
    #[arg(long, conflicts_with = "commit")]
    staged: bool,
    /// Show what one commit changed (the commit against its parent).
    #[arg(long, value_name = "REV", conflicts_with = "spec")]
    commit: Option<String>,
    /// Leave out changes that are only whitespace.
    #[arg(short = 'w', long)]
    ignore_whitespace: bool,
    /// Only list changes of this kind.
    #[arg(long, value_enum, default_value_t = KindFilter::All)]
    kind: KindFilter,
    /// Only list scopes with this access level.
    #[arg(long, value_enum, default_value_t = AccessFilter::All)]
    access: AccessFilter,
    /// Print the before and after text of one changed scope, by ID.
    #[arg(long, value_name = "ID", conflicts_with = "json")]
    scope: Option<String>,
    /// Print the full change set as JSON. Filters do not apply.
    #[arg(long)]
    json: bool,
    /// A directory inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}

#[derive(clap::Args)]
struct MapArgs {
    /// How far down the tree to show.
    #[arg(long, value_enum, default_value_t = LevelArg::Package)]
    level: LevelArg,
    /// Leave these scopes out.
    #[arg(long, value_name = "ID")]
    hide: Vec<String>,
    /// The map of a change: `REV` (against the working tree), `A..B` or
    /// `A...B`. With no value, the uncommitted changes.
    #[arg(long, value_name = "SPEC", num_args = 0..=1, default_missing_value = "")]
    compare: Option<String>,
    /// The map of what is staged, against the last commit.
    #[arg(long, conflicts_with = "commit")]
    staged: bool,
    /// The map of one commit, against its parent.
    #[arg(long, value_name = "REV", conflicts_with = "compare")]
    commit: Option<String>,
    /// Print everything as JSON, for a layout to read.
    #[arg(long)]
    json: bool,
    /// Use names and imports only, ignoring any index made by `mapdiff index`.
    #[arg(long)]
    fast: bool,
    /// The directory to map. For a change, a directory inside the
    /// repository.
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(clap::Args)]
struct ImpactArgs {
    /// What to compare: `REV` (against the working tree), `A..B`, or
    /// `A...B` (from where B branched off A).
    spec: Option<String>,
    /// Compare against what is staged instead of the working tree.
    #[arg(long, conflicts_with = "commit")]
    staged: bool,
    /// The impact of one commit (the commit against its parent).
    #[arg(long, value_name = "REV", conflicts_with = "spec")]
    commit: Option<String>,
    /// How many steps of use to follow.
    #[arg(long, default_value_t = 1)]
    depth: u32,
    /// Also count scopes whose body changed.
    #[arg(long)]
    bodies: bool,
    /// List users that are themselves part of the change too.
    #[arg(long)]
    all: bool,
    /// A directory inside the repository.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum LevelArg {
    /// Directories.
    Architecture,
    /// Files.
    Package,
    /// Types and functions, and the members of types.
    Module,
    /// Every scope.
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum KindFilter {
    /// Everything.
    All,
    /// Changes to what other code can see: signatures, and scopes added,
    /// removed, renamed or moved.
    Signatures,
    /// Changes to bodies.
    Bodies,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AccessFilter {
    All,
    Public,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return finish(serve(cli.open));
    };
    let result = match command {
        Command::Scopes { path, json } => scopes(&path, json),
        Command::Source { id, root } => source(&id, &root),
        Command::Diff(args) => diff(&args),
        Command::Refs {
            id,
            callers,
            deps,
            depth,
            json,
            fast,
            root,
        } => refs(&id, callers, deps, depth, json, tier_choice(fast), &root),
        Command::Graph { path, json, fast } => graph(&path, json, tier_choice(fast)),
        Command::Index { path } => index(&path),
        Command::Wires {
            id,
            level,
            hide,
            fast,
            root,
        } => wires(&id, level, hide, tier_choice(fast), &root),
        Command::Map(args) => map(&args),
        Command::Impact(args) => impact(&args),
        Command::Serve(open) => serve(open),
        Command::DebugTree { file } => debug_tree(&file),
        Command::DebugScip { file, symbol } => std::fs::read(&file)
            .map_err(|err| format!("cannot read {}: {err}", file.display()))
            .and_then(|bytes| mapdiff_graph::describe_scip(&bytes, symbol.as_deref()))
            .map(|text| print!("{text}")),
    };
    finish(result)
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mapdiff: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a file or walks a directory into a scope tree.
fn load(path: &Path) -> Result<ScopeTree, String> {
    if path.is_dir() {
        return mapdiff_extract::extract_repo(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()));
    }
    let source =
        std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let id = path.to_string_lossy().replace('\\', "/");
    Ok(ScopeTree::new(mapdiff_extract::extract_file(&id, &source)))
}

fn scopes(path: &Path, json: bool) -> Result<(), String> {
    let tree = load(path)?;
    if json {
        let text = serde_json::to_string_pretty(&tree).map_err(|err| err.to_string())?;
        println!("{text}");
    } else {
        print!("{}", tree.render_text());
    }
    Ok(())
}

fn source(id: &str, root: &Path) -> Result<(), String> {
    let tree = load(root)?;
    let scope = tree
        .get(id)
        .ok_or_else(|| format!("no scope with the ID {id:?} under {}", root.display()))?;
    if scope.kind == mapdiff_core::ScopeKind::Directory {
        return Err(format!("{id:?} is a directory and has no source text"));
    }
    let file = root.join(&scope.path.file);
    let bytes =
        std::fs::read(&file).map_err(|err| format!("cannot read {}: {err}", file.display()))?;
    let range = scope.range.start_byte as usize..scope.range.end_byte as usize;
    let text = bytes
        .get(range)
        .ok_or_else(|| format!("{} changed while it was being read", file.display()))?;
    println!("{}", String::from_utf8_lossy(text));
    Ok(())
}

fn diff(args: &DiffArgs) -> Result<(), String> {
    let repo = mapdiff_gitdiff::open(&args.repo).map_err(|err| err.to_string())?;
    let comparison = match &args.commit {
        Some(revision) => Comparison::commit(&repo, revision),
        None => Comparison::parse(&repo, args.spec.as_deref(), args.staged),
    }
    .map_err(|err| err.to_string())?;
    let options = Options {
        ignore_whitespace: args.ignore_whitespace,
    };
    let changes =
        mapdiff_gitdiff::change_set(&repo, &comparison, options).map_err(|err| err.to_string())?;

    if args.json {
        let text = serde_json::to_string_pretty(&changes).map_err(|err| err.to_string())?;
        println!("{text}");
        return Ok(());
    }
    if let Some(id) = &args.scope {
        return scope_diff(&repo, &comparison, &changes, id);
    }

    println!("{} -> {}", changes.base.label, changes.head.label);
    if changes.is_empty() {
        println!("no changes");
        return Ok(());
    }
    let (kind, access) = (args.kind, args.access);
    if kind == KindFilter::All && access == AccessFilter::All {
        print!("{}", changes.render_text());
    } else {
        print!(
            "{}",
            changes.render_text_where(&|change| matches_filters(change, kind, access))
        );
    }
    Ok(())
}

fn matches_filters(change: &Change, kind: KindFilter, access: AccessFilter) -> bool {
    let kind_matches = match kind {
        KindFilter::All => true,
        KindFilter::Signatures => {
            change.kinds.contains(&ChangeKind::Signature) || change.state != ChangeState::Modified
        }
        KindFilter::Bodies => change.kinds.contains(&ChangeKind::Body),
    };
    let access_matches = match access {
        AccessFilter::All => true,
        AccessFilter::Public => change.access == Access::Public,
    };
    kind_matches && access_matches
}

/// Prints the old and new text of one changed scope as a line diff.
fn scope_diff(
    repo: &mapdiff_gitdiff::Repo,
    comparison: &Comparison,
    changes: &ChangeSet,
    id: &str,
) -> Result<(), String> {
    let change = changes
        .changes
        .iter()
        .find(|change| change.new_id.as_deref() == Some(id))
        .or_else(|| {
            changes
                .changes
                .iter()
                .find(|change| change.old_id.as_deref() == Some(id))
        })
        .ok_or_else(|| format!("no changed scope with the ID {id:?}"))?;
    let (old, new) = mapdiff_gitdiff::scope_texts(repo, comparison, change);
    if old.is_none() && new.is_none() {
        return Err(format!("{id:?} has no source text"));
    }
    for line in
        mapdiff_gitdiff::diff_lines(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""))
    {
        let sign = match line.tag {
            DiffTag::Equal => ' ',
            DiffTag::Delete => '-',
            DiffTag::Insert => '+',
        };
        let text: String = line
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        println!("{sign}{text}");
    }
    Ok(())
}

fn debug_tree(file: &Path) -> Result<(), String> {
    let grammar = mapdiff_extract::Grammar::for_path(file)
        .ok_or_else(|| format!("no grammar for {}", file.display()))?;
    let source =
        std::fs::read(file).map_err(|err| format!("cannot read {}: {err}", file.display()))?;
    print!("{}", mapdiff_extract::dump_tree(grammar, &source));
    Ok(())
}

/// Keeps what a query value needs kept: letters, digits and the marks a
/// revision is written with stay readable, the rest is percent-encoded.
fn query_value(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn serve(open: OpenArgs) -> Result<(), String> {
    let git = mapdiff_gitdiff::open(&open.repo).map_err(|err| err.to_string())?;
    // Which page to open: the map, with the change that was asked for
    // drawn on it. With nothing asked for, the uncommitted changes when
    // there are any, and otherwise the map as it is.
    let page = if let Some(number) = open.pr {
        // A pull request is fetched before the server starts.
        let fetched = git
            .fetch_pull_request(number)
            .map_err(|err| err.to_string())?;
        match &fetched.title {
            Some(title) => println!("Pull request {number}: {title}"),
            None => println!("Pull request {number}, against {}", fetched.base_branch),
        }
        format!(
            "?mode=range&spec={}#map/changes",
            query_value(&fetched.spec())
        )
    } else if let Some(revision) = &open.commit {
        let commit = git.resolve(revision).map_err(|err| err.to_string())?;
        format!("?mode=commit&commit={commit}#map/changes")
    } else if open.staged {
        String::from("?mode=staged#map/changes")
    } else if let Some(what) = &open.what {
        // Checked now, so that a mistyped revision is an error here and
        // not an empty page.
        Comparison::parse(&git, Some(what), false).map_err(|err| err.to_string())?;
        format!("?mode=range&spec={}#map/changes", query_value(what))
    } else {
        let dirty = std::process::Command::new("git")
            .arg("-C")
            .arg(git.root())
            .args(["status", "--porcelain"])
            .output()
            .is_ok_and(|output| !output.stdout.is_empty());
        String::from(if dirty { "#map/changes" } else { "#map" })
    };
    let options = mapdiff_server::ServerOptions {
        repo: open.repo,
        port: open.port,
        ui_dir: open.ui_dir,
    };
    let no_open = open.no_open;
    mapdiff_server::serve(options, |address| {
        let address = format!("{address}/{page}");
        println!("mapdiff is at {address}");
        println!("Press Ctrl-C to stop.");
        if !no_open {
            open_in_browser(&address);
        }
    })
}

/// Asks the system to open an address. Failing is fine: the address was
/// printed, so it can be opened by hand.
fn open_in_browser(address: &str) {
    let (program, arguments): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    let _ = std::process::Command::new(program)
        .args(arguments)
        .arg(address)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn tier_choice(fast: bool) -> mapdiff_graph::Precise {
    if fast {
        mapdiff_graph::Precise::Off
    } else {
        mapdiff_graph::Precise::Cached
    }
}

fn build_graph(
    root: &Path,
    precise: mapdiff_graph::Precise,
) -> Result<mapdiff_graph::Graph, String> {
    mapdiff_graph::build_with(root, precise, &|message| eprintln!("{message}..."))
        .map_err(|err| format!("cannot read {}: {err}", root.display()))
}

fn tier_name(tier: mapdiff_core::Tier) -> &'static str {
    match tier {
        mapdiff_core::Tier::Fast => "fast",
        mapdiff_core::Tier::Precise => "precise",
    }
}

#[allow(clippy::too_many_arguments)]
fn refs(
    id: &str,
    callers: bool,
    deps: bool,
    depth: u32,
    json: bool,
    precise: mapdiff_graph::Precise,
    root: &Path,
) -> Result<(), String> {
    let graph = build_graph(root, precise)?;
    let inside = graph.tree.subtree_ids(id);
    if inside.is_empty() {
        return Err(format!(
            "no scope with the ID {id:?} under {}",
            root.display()
        ));
    }
    let references = &graph.references;
    let (show_callers, show_deps) = (!deps, !callers);

    if json {
        let mut out = serde_json::Map::new();
        out.insert(String::from("id"), serde_json::json!(id));
        if show_callers {
            let found: Vec<_> = references.incoming(&inside).collect();
            out.insert(String::from("used_by"), serde_json::json!(found));
        }
        if show_deps {
            let found: Vec<_> = references.outgoing(&inside).collect();
            out.insert(String::from("uses"), serde_json::json!(found));
        }
        let text = serde_json::to_string_pretty(&out).map_err(|err| err.to_string())?;
        println!("{text}");
        return Ok(());
    }

    println!("{id}");
    let direct = |title: &str,
                  found: Vec<&mapdiff_core::Reference>,
                  other: fn(&mapdiff_core::Reference) -> &str| {
        println!("{title} ({}):", found.len());
        for reference in found {
            // The reference is written in the file of the scope that makes it.
            let file = graph
                .tree
                .get(&reference.from)
                .map_or("", |scope| scope.path.file.as_str());
            println!(
                "  {}  [{}, {}]  {file}:{}",
                other(reference),
                ref_kind_name(reference.kind),
                tier_name(reference.tier),
                reference.range.start.line + 1
            );
        }
    };
    let by_distance = |title: &str, found: Vec<(u32, String)>| {
        println!("{title}, up to {depth} steps away ({}):", found.len());
        for (distance, scope) in found {
            println!("  {distance}  {scope}");
        }
    };
    if show_callers {
        if depth <= 1 {
            direct(
                "used by",
                references.incoming(&inside).collect(),
                |reference| &reference.from,
            );
        } else {
            by_distance("used by", references.callers(&inside, depth));
        }
    }
    if show_deps {
        if depth <= 1 {
            direct(
                "uses",
                references.outgoing(&inside).collect(),
                |reference| &reference.to,
            );
        } else {
            by_distance("uses", references.dependencies(&inside, depth));
        }
    }
    Ok(())
}

fn graph(path: &Path, json: bool, precise: mapdiff_graph::Precise) -> Result<(), String> {
    let graph = build_graph(path, precise)?;
    if json {
        let text =
            serde_json::to_string_pretty(&graph.references).map_err(|err| err.to_string())?;
        println!("{text}");
        return Ok(());
    }
    print_summary(&graph);
    Ok(())
}

fn index(path: &Path) -> Result<(), String> {
    let graph = build_graph(path, mapdiff_graph::Precise::Refresh)?;
    if graph.tiers.is_empty() {
        println!("no Rust, Python, TypeScript or JavaScript sources to index");
    }
    print_summary(&graph);
    Ok(())
}

fn print_summary(graph: &mapdiff_graph::Graph) {
    let stats = &graph.references.stats;
    let precise = graph
        .references
        .references
        .iter()
        .filter(|reference| reference.tier == mapdiff_core::Tier::Precise)
        .count();
    println!(
        "{} references between scopes, from {} source files ({precise} precise, {} fast)",
        stats.references,
        stats.files,
        stats.references as usize - precise
    );
    for (kind, count) in &stats.by_kind {
        println!("  {:<10} {count}", ref_kind_name(*kind));
    }

    for tier in &graph.tiers {
        let place = if tier.directory == "." {
            String::new()
        } else {
            format!(" in {}", tier.directory)
        };
        match &tier.outcome {
            mapdiff_graph::Outcome::Used {
                stats,
                from_cache,
                seconds,
            } => {
                let when = if *from_cache {
                    String::from("index reused")
                } else {
                    format!("indexed in {seconds:.1} s")
                };
                println!(
                    "{}{place}: {when}; {} files, {} uses of names, {} became references \
                     ({} are of code outside the repository, {} of parameters and other non-scopes)",
                    tier.tool,
                    stats.documents,
                    stats.uses,
                    stats.references,
                    stats.external,
                    stats.not_a_scope
                );
            }
            mapdiff_graph::Outcome::NotIndexed => {
                println!(
                    "{}{place}: no index for the files as they are now; run `mapdiff index`",
                    tier.tool
                );
            }
            mapdiff_graph::Outcome::ToolMissing { install } => {
                println!("{}{place}: not installed ({install})", tier.tool);
            }
            mapdiff_graph::Outcome::Failed(message) => println!("{}{place}: {message}", tier.tool),
        }
    }

    let comparison = &graph.comparison;
    if comparison.precise > 0 {
        let share = 100.0 * f64::from(comparison.agreed) / f64::from(comparison.precise);
        println!(
            "fast tier against precise: found {} of {} precise references ({share:.0}%); \
             {} fast references were wrong and dropped; {} kept where the indexers say nothing",
            comparison.agreed, comparison.precise, comparison.contradicted, comparison.fast_only
        );
        if comparison.overruled > 0 {
            println!(
                "{} precise answers were set aside: they point at a scope with another name \
                 than the one written, where the fast tier points at one with that name",
                comparison.overruled
            );
        }
        if comparison.unchecked_renames > 0 {
            println!(
                "{} precise references point at a scope with another name than the one written, \
                 with no fast reference to check them against (aliases, mostly)",
                comparison.unchecked_renames
            );
        }
    } else {
        println!(
            "{} names could not be resolved (standard library, third-party code, or values whose type is needed)",
            stats.unresolved
        );
    }
}

fn ref_kind_name(kind: mapdiff_core::RefKind) -> &'static str {
    match kind {
        mapdiff_core::RefKind::Call => "call",
        mapdiff_core::RefKind::TypeUse => "type",
        mapdiff_core::RefKind::Import => "import",
        mapdiff_core::RefKind::Inherit => "inherit",
        mapdiff_core::RefKind::Implement => "implement",
        mapdiff_core::RefKind::Macro => "macro",
        mapdiff_core::RefKind::Access => "access",
    }
}

fn wires(
    id: &str,
    level: LevelArg,
    hide: Vec<String>,
    precise: mapdiff_graph::Precise,
    root: &Path,
) -> Result<(), String> {
    let graph = build_graph(root, precise)?;
    if graph.tree.get(id).is_none() {
        return Err(format!(
            "no scope with the ID {id:?} under {}",
            root.display()
        ));
    }
    let lifted = graph.lift(&mapdiff_graph::View::level(level.into()).hiding(hide));
    let empty = mapdiff_graph::ContainerWires::default();
    let container = lifted.containers.get(id).unwrap_or(&empty);

    // Children are named by what comes after the container's own ID.
    let short = |child: &str| -> String {
        let rest = match id {
            "." => child,
            _ => child.strip_prefix(id).unwrap_or(child),
        };
        rest.trim_start_matches("::")
            .trim_start_matches('/')
            .to_owned()
    };

    println!("{id}");
    println!("wires among its children ({}):", container.wires.len());
    for wire in &container.wires {
        let kind = match wire.kind {
            mapdiff_core::WireKind::Direct => "",
            mapdiff_core::WireKind::Cycle => "  (closes a loop)",
            mapdiff_core::WireKind::Bridge => "  (through hidden scopes)",
        };
        println!(
            "  {} -> {}  x{}{kind}",
            short(&wire.from),
            short(&wire.to),
            wire.count
        );
    }
    println!("used from outside ({}):", container.entries.len());
    for entry in &container.entries {
        println!(
            "  {}  {} references from {} places",
            short(&entry.id),
            entry.incoming,
            entry.from
        );
    }
    println!("use the outside ({}):", container.exits.len());
    for exit in &container.exits {
        println!(
            "  {}  {} references to {} places",
            short(&exit.id),
            exit.outgoing,
            exit.to
        );
    }
    Ok(())
}

impl From<LevelArg> for mapdiff_graph::Level {
    fn from(level: LevelArg) -> Self {
        match level {
            LevelArg::Architecture => mapdiff_graph::Level::Architecture,
            LevelArg::Package => mapdiff_graph::Level::Package,
            LevelArg::Module => mapdiff_graph::Level::Module,
            LevelArg::Code => mapdiff_graph::Level::Code,
        }
    }
}

fn map(args: &MapArgs) -> Result<(), String> {
    let hidden: std::collections::HashSet<String> = args.hide.iter().cloned().collect();
    let precise = tier_choice(args.fast);
    let compared = args.compare.is_some() || args.staged || args.commit.is_some();
    let data = if compared {
        let repo = mapdiff_gitdiff::open(&args.root).map_err(|err| err.to_string())?;
        let spec = args.compare.as_deref().filter(|spec| !spec.is_empty());
        let comparison = match &args.commit {
            Some(revision) => Comparison::commit(&repo, revision),
            None => Comparison::parse(&repo, spec, args.staged),
        }
        .map_err(|err| err.to_string())?;
        let changes = mapdiff_gitdiff::change_set(&repo, &comparison, Options::default())
            .map_err(|err| err.to_string())?;
        mapdiff_graph::comparison_map(
            &repo,
            &comparison,
            &changes,
            args.level.into(),
            &hidden,
            precise,
        )
        .map_err(|err| err.to_string())?
    } else {
        build_graph(&args.root, precise)?.map(&args.root, args.level.into(), &hidden)
    };

    if args.json {
        let text = serde_json::to_string(&data).map_err(|err| err.to_string())?;
        println!("{text}");
        return Ok(());
    }
    let count = |presence: mapdiff_graph::Presence| {
        data.nodes
            .iter()
            .filter(|node| node.presence == presence)
            .count()
    };
    let wires: usize = data
        .containers
        .values()
        .map(|inside| inside.wires.len())
        .sum();
    println!(
        "{} boxes, {} wires in {} containers",
        data.nodes.len(),
        wires,
        data.containers.len()
    );
    if compared {
        println!(
            "{} boxes added, {} only there before (kept as ghosts)",
            count(mapdiff_graph::Presence::Added),
            count(mapdiff_graph::Presence::Removed)
        );
    }
    Ok(())
}

fn impact(args: &ImpactArgs) -> Result<(), String> {
    let repo = mapdiff_gitdiff::open(&args.repo).map_err(|err| err.to_string())?;
    let comparison = match &args.commit {
        Some(revision) => Comparison::commit(&repo, revision),
        None => Comparison::parse(&repo, args.spec.as_deref(), args.staged),
    }
    .map_err(|err| err.to_string())?;
    let changes = mapdiff_gitdiff::change_set(&repo, &comparison, Options::default())
        .map_err(|err| err.to_string())?;

    // The graph after the change, and the one before it, which is only
    // needed for scopes that are gone.
    let head = mapdiff_graph::checkout(
        &repo,
        comparison.head_side(),
        mapdiff_graph::Precise::Cached,
    )
    .map_err(|err| err.to_string())?;
    let any_removed = changes
        .changes
        .iter()
        .any(|change| change.is_scope() && change.state == mapdiff_core::ChangeState::Removed);
    let base = match any_removed {
        true => Some(
            mapdiff_graph::checkout(&repo, &comparison.base_side(), mapdiff_graph::Precise::Off)
                .map_err(|err| err.to_string())?,
        ),
        false => None,
    };

    let options = mapdiff_graph::ImpactOptions {
        depth: args.depth,
        bodies: args.bodies,
    };
    let report = mapdiff_graph::impact(
        &changes,
        &head.graph,
        base.as_ref().map(|side| &side.graph),
        options,
    );

    println!("{} -> {}", changes.base.label, changes.head.label);
    println!(
        "{} scopes changed in a way their users can see; {} unchanged scopes use them",
        report.visible_changes, report.unchanged_users
    );
    let updated: usize = report
        .impacted
        .iter()
        .flat_map(|impacted| &impacted.affected)
        .filter(|user| user.also_changed)
        .count();
    if updated > 0 && !args.all {
        println!("{updated} uses are by scopes that are part of the change (--all lists them)");
    }
    if report.removed_not_checked {
        println!("scopes were removed, but who used them could not be looked up");
    }
    for impacted in &report.impacted {
        let shown: Vec<&mapdiff_graph::Affected> = impacted
            .affected
            .iter()
            .filter(|user| args.all || !user.also_changed)
            .collect();
        let hidden = impacted.affected.len() - shown.len();
        if shown.is_empty() && !args.all {
            continue;
        }
        let reasons: Vec<&str> = impacted
            .reasons
            .iter()
            .map(|reason| reason.describe())
            .collect();
        println!("{}  ({})", impacted.id, reasons.join(", "));
        for user in shown {
            let place = user.references.first().map_or(String::new(), |reference| {
                format!("  line {}", reference.range.start.line + 1)
            });
            let note = if user.also_changed {
                "  (also changed)"
            } else {
                ""
            };
            println!("  {}  {}{place}{note}", user.distance, user.id);
        }
        if hidden > 0 {
            println!("  and {hidden} users that are part of the change (--all lists them)");
        }
    }
    Ok(())
}
