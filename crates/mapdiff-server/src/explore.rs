//! What the map viewer asks about the files as they are now: who uses a
//! scope, the highlighted text of a file, and where a name is.
//!
//! The graph is built once and kept until a file changes.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use mapdiff_core::{Access, RefKind, Reference, Scope, ScopeKind};
use mapdiff_extract::highlight::highlight;
use mapdiff_extract::parse::Grammar;
use mapdiff_graph::{Graph, Level};
use serde::{Deserialize, Serialize};

use crate::detail::{RowSpan, spans};
use crate::{ApiError, ApiResult, AppState, DiffQuery, Loaded, blocking, load};

pub struct Live {
    stamp: u64,
    pub graph: Graph,
}

/// A comparison with both of its sides on disk and their graphs. Kept
/// while the page keeps asking about the same comparison.
pub struct Shown {
    key: String,
    pub loaded: Arc<Loaded>,
    pub compared: mapdiff_graph::Compared,
}

/// What a request is about: the files as they are, or a comparison.
pub enum Subject {
    Live(Arc<Live>),
    Shown(Arc<Shown>),
}

impl Subject {
    /// The graph and the files after the change, or as they are now.
    pub fn head<'a>(&'a self, state: &'a AppState) -> (&'a Graph, &'a Path) {
        match self {
            Subject::Live(live) => (&live.graph, state.repo.root()),
            Subject::Shown(shown) => (&shown.compared.head.graph, &shown.compared.head.root),
        }
    }

    /// The graph and the files before the change.
    pub fn base(&self) -> Option<(&Graph, &Path)> {
        match self {
            Subject::Live(_) => None,
            Subject::Shown(shown) => Some((&shown.compared.base.graph, &shown.compared.base.root)),
        }
    }
}

/// Which comparison a request is about, as its query says. Every route of
/// the map takes these four.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Which {
    /// Without this the request is about the files as they are.
    #[serde(default)]
    pub compare: bool,
    pub spec: Option<String>,
    pub commit: Option<String>,
    #[serde(default)]
    pub staged: bool,
}

impl Which {
    fn query(&self) -> DiffQuery {
        DiffQuery {
            spec: self.spec.clone(),
            commit: self.commit.clone(),
            staged: self.staged,
            w: false,
        }
    }
}

pub fn subject(state: &AppState, which: &Which) -> Result<Subject, ApiError> {
    match which.compare {
        true => shown(state, &which.query()).map(Subject::Shown),
        false => live(state).map(Subject::Live),
    }
}

/// The comparison a query names, with both sides on disk. Built once and
/// kept until another comparison is asked for, or, for one that involves
/// the working tree or the index, until those change.
pub fn shown(state: &AppState, query: &DiffQuery) -> Result<Arc<Shown>, ApiError> {
    let failed = |err: String| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err);
    let root = state.repo.root();
    let mut kept = state.shown.lock().unwrap();
    // The files and the index can change under a comparison that reads
    // them, so their state is part of what names it. For one between two
    // commits this is wasted but cheap.
    let stamp = mapdiff_extract::stamp(root).map_err(|err| failed(err.to_string()))?;
    let index = state
        .repo
        .git_dir()
        .ok()
        .and_then(|directory| std::fs::metadata(directory.join("index")).ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    let key = format!("{}|{stamp}|{index}", query.key());
    if let Some(shown) = kept.as_ref()
        && shown.key == key
    {
        return Ok(shown.clone());
    }
    let loaded = load(state, query)?;
    let compared = mapdiff_graph::compare(
        &state.repo,
        &loaded.comparison,
        mapdiff_graph::Precise::Cached,
    )
    .map_err(|err| failed(err.to_string()))?;
    let shown = Arc::new(Shown {
        key,
        loaded,
        compared,
    });
    *kept = Some(shown.clone());
    Ok(shown)
}

/// The graph of the working tree, rebuilt only when a file was added,
/// removed or written to since the last call.
pub fn live(state: &AppState) -> Result<Arc<Live>, ApiError> {
    let failed = |err: std::io::Error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    let root = state.repo.root();
    // Held while building, so that two requests arriving together do not
    // both build.
    let mut kept = state.live.lock().unwrap();
    let stamp = mapdiff_extract::stamp(root).map_err(failed)?;
    if let Some(live) = kept.as_ref()
        && live.stamp == stamp
    {
        return Ok(live.clone());
    }
    let graph =
        mapdiff_graph::build_with(root, mapdiff_graph::Precise::Cached, &|_| {}).map_err(failed)?;
    let live = Arc::new(Live { stamp, graph });
    *kept = Some(live.clone());
    Ok(live)
}

pub fn parse_level(level: Option<&str>) -> Result<Level, ApiError> {
    match level.unwrap_or("package") {
        "architecture" => Ok(Level::Architecture),
        "package" => Ok(Level::Package),
        "module" => Ok(Level::Module),
        "code" => Ok(Level::Code),
        other => Err(ApiError::bad_request(format!("no level called {other:?}"))),
    }
}

/// What a box is titled when its scope has no name of its own.
fn label_of(scope: &Scope) -> String {
    scope.name.clone().unwrap_or_else(|| {
        scope
            .path
            .segments
            .last()
            .cloned()
            .unwrap_or_else(|| scope.path.file.clone())
    })
}

#[derive(Deserialize)]
pub struct NodeQuery {
    id: String,
    level: Option<String>,
    #[serde(default)]
    compare: bool,
    spec: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    staged: bool,
}

/// One place where a name is used.
#[derive(Serialize)]
pub struct Place {
    path: String,
    /// Counted from 1.
    line: u32,
    /// Counted from 0, in characters.
    column: u32,
    end_column: u32,
    kind: RefKind,
    /// The line of source, without the indentation.
    text: String,
    /// How many characters of indentation were taken off `text`.
    indent: u32,
}

/// Another scope that uses this one, or that this one uses.
#[derive(Serialize)]
pub struct Use {
    id: String,
    /// The box it is drawn in at the level asked for.
    at: String,
    kind: ScopeKind,
    lang_kind: String,
    label: String,
    path: String,
    count: u32,
    /// Where: in the other scope for a caller, in this scope for a
    /// dependency. The first few.
    places: Vec<Place>,
}

#[derive(Serialize)]
pub struct Crumb {
    id: String,
    label: String,
}

#[derive(Serialize)]
pub struct NodeInfo {
    /// `head`: the scope as it is after the change, or now. `base`: a
    /// scope that is only there before the change.
    side: &'static str,
    id: String,
    kind: ScopeKind,
    lang_kind: String,
    label: String,
    signature: Option<String>,
    access: Access,
    path: String,
    /// Counted from 1. 0 for a directory.
    first_line: u32,
    last_line: u32,
    /// From the repository root down to the scope's parent.
    ancestors: Vec<Crumb>,
    callers: Vec<Use>,
    dependencies: Vec<Use>,
}

/// How many places are listed for each scope on the other end.
const PLACES: usize = 12;

struct Lines<'a> {
    root: &'a Path,
    files: HashMap<String, Vec<String>>,
}

impl Lines<'_> {
    fn place(&mut self, path: &str, reference: &Reference) -> Place {
        let lines = self.files.entry(path.to_owned()).or_insert_with(|| {
            std::fs::read_to_string(self.root.join(path))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        });
        let range = reference.range;
        let line = lines
            .get(range.start.line as usize)
            .map(String::as_str)
            .unwrap_or("");
        // Columns in the model count bytes; a page counts characters.
        let characters = |byte: u32| {
            line.char_indices()
                .take_while(|(at, _)| (*at as u32) < byte)
                .count() as u32
        };
        let column = characters(range.start.column);
        let end_column = match range.end.line == range.start.line {
            true => characters(range.end.column),
            false => line.chars().count() as u32,
        };
        let text = line.trim_start();
        Place {
            path: path.to_owned(),
            line: range.start.line + 1,
            column,
            end_column,
            kind: reference.kind,
            text: text.trim_end().to_owned(),
            indent: (line.chars().count() - text.chars().count()) as u32,
        }
    }
}

/// A scope, with the scopes that use it and the scopes it uses.
pub async fn node(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NodeQuery>,
) -> ApiResult<NodeInfo> {
    blocking(move || {
        let level = parse_level(query.level.as_deref())?;
        let which = Which {
            compare: query.compare,
            spec: query.spec.clone(),
            commit: query.commit.clone(),
            staged: query.staged,
        };
        let subject = subject(&state, &which)?;
        // A box that is only there before the change is looked up there.
        // Its ID may carry a mark that sets it apart from a new scope of
        // the same name.
        let before_id = query.id.strip_suffix(" (before)").unwrap_or(&query.id);
        let (graph, root, id, side) = match subject.head(&state) {
            (graph, root) if graph.tree.get(&query.id).is_some() => {
                (graph, root, query.id.as_str(), "head")
            }
            _ => match subject.base() {
                Some((graph, root)) if graph.tree.get(before_id).is_some() => {
                    (graph, root, before_id, "base")
                }
                _ => {
                    return Err(ApiError(
                        StatusCode::NOT_FOUND,
                        format!("no scope with the ID {:?}", query.id),
                    ));
                }
            },
        };
        let scope = graph.tree.get(id).expect("found just above");
        let by_id: HashMap<&str, &Scope> = graph
            .tree
            .scopes
            .iter()
            .map(|scope| (scope.id.as_str(), scope))
            .collect();
        let lifter = graph.lifter();
        let inside = graph.tree.subtree_ids(id);
        let mut lines = Lines {
            root,
            files: HashMap::new(),
        };

        // Grouped by the scope on the other end, as the code level draws
        // it: a use inside a nested function counts for the outer one.
        let mut gather = |references: Vec<&Reference>, callers: bool| -> Vec<Use> {
            let mut groups: BTreeMap<&str, Vec<&Reference>> = BTreeMap::new();
            for reference in references {
                let other = match callers {
                    true => reference.from.as_str(),
                    false => reference.to.as_str(),
                };
                let Some(other) = lifter.at(other, Level::Code) else {
                    continue;
                };
                groups.entry(other).or_default().push(reference);
            }
            let mut uses: Vec<Use> = groups
                .into_iter()
                .filter_map(|(other, references)| {
                    let other_scope = by_id.get(other)?;
                    let places = references
                        .iter()
                        .take(PLACES)
                        .filter_map(|reference| {
                            // A reference is written in the file of its
                            // `from` end.
                            let file = &by_id.get(reference.from.as_str())?.path.file;
                            Some(lines.place(file, reference))
                        })
                        .collect();
                    Some(Use {
                        id: other.to_owned(),
                        at: lifter.at(other, level).unwrap_or(other).to_owned(),
                        kind: other_scope.kind,
                        lang_kind: other_scope.lang_kind.clone(),
                        label: label_of(other_scope),
                        path: other_scope.path.file.clone(),
                        count: references.len() as u32,
                        places,
                    })
                })
                .collect();
            uses.sort_by(|a, b| b.count.cmp(&a.count).then(a.id.cmp(&b.id)));
            uses
        };
        let callers = gather(graph.references.incoming(&inside).collect(), true);
        let dependencies = gather(graph.references.outgoing(&inside).collect(), false);

        let mut ancestors = Vec::new();
        let mut current = scope.parent.as_deref();
        while let Some(parent) = current.and_then(|id| by_id.get(id)) {
            ancestors.push(Crumb {
                id: parent.id.clone(),
                label: label_of(parent),
            });
            current = parent.parent.as_deref();
        }
        ancestors.reverse();

        let is_directory = scope.kind == ScopeKind::Directory;
        let last_line = match scope.range.end.column {
            0 if scope.range.end.line > scope.range.start.line => scope.range.end.line,
            _ => scope.range.end.line + 1,
        };
        Ok(Json(NodeInfo {
            side,
            id: query.id.clone(),
            kind: scope.kind,
            lang_kind: scope.lang_kind.clone(),
            label: label_of(scope),
            signature: scope.signature.clone(),
            access: scope.access,
            path: scope.path.file.clone(),
            first_line: if is_directory {
                0
            } else {
                scope.range.start.line + 1
            },
            last_line: if is_directory { 0 } else { last_line },
            ancestors,
            callers,
            dependencies,
        }))
    })
    .await
}

#[derive(Deserialize)]
pub struct CodeQuery {
    path: String,
    /// `base` for the file as it was before the change.
    side: Option<String>,
    #[serde(default)]
    compare: bool,
    spec: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    staged: bool,
}

#[derive(Serialize)]
pub struct Code {
    path: String,
    /// One list of spans per line of the file.
    lines: Vec<Vec<RowSpan>>,
}

/// Files larger than this are sent without colours.
const HIGHLIGHT_LIMIT: usize = 1_500_000;

/// The text of one file, split into lines and highlighted. The map draws
/// each scope's code from the lines its range covers.
pub async fn code(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CodeQuery>,
) -> ApiResult<Code> {
    blocking(move || {
        let which = Which {
            compare: query.compare,
            spec: query.spec.clone(),
            commit: query.commit.clone(),
            staged: query.staged,
        };
        let subject = subject(&state, &which)?;
        let (graph, root) = match query.side.as_deref() {
            Some("base") => subject.base().ok_or_else(|| {
                ApiError::bad_request("there is no side before the change without a comparison")
            })?,
            _ => subject.head(&state),
        };
        // Only files the tree knows: the path is never trusted as a path.
        let known = graph
            .tree
            .get(&query.path)
            .is_some_and(|scope| matches!(scope.kind, ScopeKind::File | ScopeKind::Document));
        if !known {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                format!("no file called {:?}", query.path),
            ));
        }
        let source = std::fs::read(root.join(&query.path)).unwrap_or_default();
        let text = String::from_utf8_lossy(&source);
        let highlights = match Grammar::for_path(Path::new(&query.path)) {
            Some(grammar)
                if source.len() <= HIGHLIGHT_LIMIT && std::str::from_utf8(&source).is_ok() =>
            {
                highlight(grammar, &source)
            }
            _ => Vec::new(),
        };
        let lines = text
            .lines()
            .enumerate()
            .map(|(number, line)| {
                let colours = highlights.get(number).map(Vec::as_slice).unwrap_or(&[]);
                spans(line.trim_end_matches('\r'), colours, &[])
            })
            .collect();
        Ok(Json(Code {
            path: query.path,
            lines,
        }))
    })
    .await
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    limit: Option<usize>,
}

#[derive(Serialize)]
pub struct Hit {
    id: String,
    /// The box it is drawn in at the code level: itself, or the function
    /// around it.
    at: String,
    /// The first level, from the top, that shows `at` as a box.
    level: Level,
    kind: ScopeKind,
    lang_kind: String,
    label: String,
    path: String,
}

#[derive(Serialize)]
pub struct Hits {
    hits: Vec<Hit>,
}

/// How well `query` matches `text` as letters in order, not necessarily
/// next to each other. Higher is better; `None` when a letter is missing.
/// Both must be lower case already.
fn fuzzy(query: &[char], text: &str) -> Option<i32> {
    let text: Vec<char> = text.chars().collect();
    let first = *query.first()?;
    // Taking each letter at its first chance can miss a better match
    // further on, so every place the first letter occurs is tried.
    (0..text.len())
        .filter(|start| text[*start] == first)
        .filter_map(|start| fuzzy_from(query, &text, start))
        .max()
}

fn fuzzy_from(query: &[char], text: &[char], start: usize) -> Option<i32> {
    let mut score = 0;
    let mut wanted = query.iter().peekable();
    let mut run = 0;
    for at in start..text.len() {
        let Some(next) = wanted.peek() else { break };
        if text[at] == **next {
            wanted.next();
            run += 1;
            // Letters in a row, and letters that start a word, count more.
            score += 1 + run * 2;
            if at == 0 || !text[at - 1].is_alphanumeric() {
                score += 6;
            }
        } else {
            run = 0;
        }
    }
    wanted.peek().is_none().then_some(score)
}

/// Scopes whose name or ID matches what was typed, best first.
pub async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Hits> {
    blocking(move || {
        let live = live(&state)?;
        let wanted: Vec<char> = query
            .q
            .to_lowercase()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if wanted.is_empty() {
            return Ok(Json(Hits { hits: Vec::new() }));
        }
        let lifter = live.graph.lifter();
        let mut scored: Vec<(i32, &Scope)> = live
            .graph
            .tree
            .scopes
            .iter()
            .skip(1)
            // Imports and loose statements have no name to look for.
            .filter(|scope| !matches!(scope.kind, ScopeKind::Import | ScopeKind::Block))
            .filter_map(|scope| {
                let label = label_of(scope).to_lowercase();
                // A match in the name beats one spread over the path.
                let in_label = fuzzy(&wanted, &label).map(|score| score * 3 + 40);
                let in_id = fuzzy(&wanted, &scope.id.to_lowercase());
                let score = in_label.max(in_id)?;
                // Of two equal matches, the shorter name is the closer one.
                Some((score * 4 - label.chars().count() as i32, scope))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        let hits = scored
            .into_iter()
            .take(query.limit.unwrap_or(30).min(200))
            .filter_map(|(_, scope)| {
                let at = lifter.at(&scope.id, Level::Code)?;
                Some(Hit {
                    id: scope.id.clone(),
                    at: at.to_owned(),
                    level: lifter.shallowest(at)?,
                    kind: scope.kind,
                    lang_kind: scope.lang_kind.clone(),
                    label: label_of(scope),
                    path: scope.path.file.clone(),
                })
            })
            .collect();
        Ok(Json(Hits { hits }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::fuzzy;

    fn score(query: &str, text: &str) -> Option<i32> {
        let query: Vec<char> = query.chars().collect();
        fuzzy(&query, text)
    }

    #[test]
    fn letters_must_all_be_there_in_order() {
        assert!(score("mde", "matched_dir_entry").is_some());
        assert!(score("edm", "matched_dir_entry").is_none());
        assert!(score("xyz", "matched_dir_entry").is_none());
    }

    #[test]
    fn word_starts_and_runs_count_more() {
        // The three word starts beat three letters from the middle.
        assert!(score("mde", "matched_dir_entry") > score("ace", "matched_dir_entry"));
        // A run beats the same letters apart.
        assert!(score("dir", "matched_dir_entry") > score("dir", "daxixr"));
        // The later, better place is found.
        assert_eq!(score("dir", "matched_dir_entry"), score("dir", "dir"));
    }
}

#[derive(Deserialize)]
pub struct PullRequestQuery {
    number: u32,
}

#[derive(Serialize)]
pub struct PullRequestReply {
    number: u32,
    title: Option<String>,
    base_branch: String,
    /// What to compare to see what the pull request changes.
    spec: String,
}

/// Fetches a GitHub pull request from `origin`, so that it can be opened
/// as a comparison. This is the one request that reaches the network and
/// adds to the repository: two refs under `refs/mapdiff/pr/`.
pub async fn pull_request(
    State(state): State<Arc<AppState>>,
    Json(query): Json<PullRequestQuery>,
) -> ApiResult<PullRequestReply> {
    blocking(move || {
        let fetched = state
            .repo
            .fetch_pull_request(query.number)
            .map_err(|err| ApiError(StatusCode::BAD_GATEWAY, err.to_string()))?;
        Ok(Json(PullRequestReply {
            number: fetched.number,
            spec: fetched.spec(),
            title: fetched.title,
            base_branch: fetched.base_branch,
        }))
    })
    .await
}
