//! The local HTTP API the reviewer page talks to, and the page itself.
//!
//! It listens on the loopback address only and reads the repository it was
//! started in. In the repository it writes review marks under
//! `.git/mapdiff/`; outside it, a note that it is running and any indexes
//! it is asked to make, in mapdiff's cache folder.

mod detail;
mod explore;
mod indexing;
mod reviewed;
pub mod running;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use mapdiff_core::ChangeSet;
use mapdiff_gitdiff::{Comparison, Options, Repo};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use detail::ChangeDetail;
use reviewed::ReviewedStore;

pub struct ServerOptions {
    /// A directory inside the repository to review.
    pub repo: PathBuf,
    /// The first port to try. The next few are tried when it is taken.
    pub port: u16,
    /// Where the built page is. `None` uses the default location.
    pub ui_dir: Option<PathBuf>,
    /// What it was started to show, for `mapdiff list`.
    pub showing: String,
    /// Run the language indexers in the background, now and whenever the
    /// page asks again after files changed.
    pub index: bool,
}

pub struct AppState {
    repo: Repo,
    reviewed: ReviewedStore,
    /// Serialises writes to the review marks file.
    reviewed_lock: Mutex<()>,
    /// The last change set between two commits. It cannot go stale, so the
    /// before and after of each scope is served without rebuilding it.
    cache: Mutex<Option<(String, Arc<Loaded>)>>,
    /// The graph of the files as they are now, kept until one changes.
    live: Mutex<Option<Arc<explore::Live>>>,
    /// The last comparison drawn on the map, with both sides on disk.
    shown: Mutex<Option<Arc<explore::Shown>>>,
    indexing: Mutex<indexing::Indexing>,
    /// Whether the indexers are started without being asked.
    auto_index: bool,
    /// The note that this server is running, once it is listening.
    announced: Mutex<Option<running::Running>>,
}

struct Loaded {
    comparison: Comparison,
    changes: ChangeSet,
}

impl AppState {
    pub fn new(repo_path: &Path) -> Result<Self, String> {
        let repo = mapdiff_gitdiff::open(repo_path).map_err(|err| err.to_string())?;
        let git_dir = repo.git_dir().map_err(|err| err.to_string())?;
        Ok(Self {
            repo,
            reviewed: ReviewedStore::new(&git_dir),
            reviewed_lock: Mutex::new(()),
            cache: Mutex::new(None),
            live: Mutex::new(None),
            shown: Mutex::new(None),
            indexing: Mutex::default(),
            auto_index: false,
            announced: Mutex::new(None),
        })
    }

    /// Keeps the note of this server saying what it draws now.
    fn note_showing(&self, showing: &str) {
        let mut announced = self.announced.lock().unwrap();
        if let Some(running) = announced.as_mut()
            && running.showing != showing
        {
            running.showing = showing.to_owned();
            running::announce(&mapdiff_graph::cache_home(), running);
        }
    }
}

/// An error as the page receives it: a status and a sentence.
struct ApiError(StatusCode, String);

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

/// Runs work that blocks (git, parsing) off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
}

/// What to compare, as the page asks for it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiffQuery {
    /// `REV`, `A..B` or `A...B`. Nothing means the last commit.
    spec: Option<String>,
    /// One commit against its parent. Overrides `spec`.
    commit: Option<String>,
    /// Compare against the index instead of the working tree.
    #[serde(default)]
    staged: bool,
    /// Leave out whitespace-only changes.
    #[serde(default)]
    w: bool,
}

impl DiffQuery {
    fn key(&self) -> String {
        format!(
            "{:?}|{:?}|{}|{}",
            self.spec, self.commit, self.staged, self.w
        )
    }

    /// The comparison in a few words, for `mapdiff list`.
    fn describe(&self) -> String {
        let given = |text: &Option<String>| {
            text.as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        if let Some(commit) = given(&self.commit) {
            return format!("commit {}", commit.chars().take(7).collect::<String>());
        }
        if self.staged {
            return String::from("staged");
        }
        match given(&self.spec) {
            // A fetched pull request is compared by the refs it was kept under.
            Some(spec) => match spec.rsplit_once("refs/mapdiff/pr/") {
                Some((_, number)) if number.chars().all(|digit| digit.is_ascii_digit()) => {
                    format!("pull request {number}")
                }
                _ => spec,
            },
            None => String::from("uncommitted"),
        }
    }
}

fn load(state: &AppState, query: &DiffQuery) -> Result<Arc<Loaded>, ApiError> {
    let key = query.key();
    if let Some((cached_key, loaded)) = state.cache.lock().unwrap().as_ref()
        && *cached_key == key
    {
        return Ok(loaded.clone());
    }

    let spec = query.spec.as_deref().filter(|spec| !spec.trim().is_empty());
    let comparison = match query
        .commit
        .as_deref()
        .filter(|commit| !commit.trim().is_empty())
    {
        Some(commit) => Comparison::commit(&state.repo, commit),
        None => Comparison::parse(&state.repo, spec, query.staged),
    }
    .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let options = Options {
        ignore_whitespace: query.w,
    };
    let changes = mapdiff_gitdiff::change_set(&state.repo, &comparison, options)
        .map_err(|err| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let fixed = comparison.is_fixed();
    let loaded = Arc::new(Loaded {
        comparison,
        changes,
    });
    // A comparison that involves the working tree or the index can change
    // at any moment, so only commit-to-commit results are kept.
    *state.cache.lock().unwrap() = fixed.then(|| (key, loaded.clone()));
    Ok(loaded)
}

#[derive(Serialize)]
struct Meta {
    /// The repository folder's name.
    name: String,
    root: String,
    branch: Option<String>,
    default_branch: Option<String>,
    branches: Vec<String>,
    commits: Vec<CommitDto>,
}

#[derive(Serialize)]
struct CommitDto {
    hash: String,
    short: String,
    subject: String,
    author: String,
    date: String,
}

async fn meta(State(state): State<Arc<AppState>>) -> ApiResult<Meta> {
    blocking(move || {
        let repo = &state.repo;
        let root = repo.root();
        Ok(Json(Meta {
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            root: root.to_string_lossy().into_owned(),
            branch: repo.head_branch(),
            default_branch: repo.default_branch(),
            branches: repo.branches(),
            commits: repo
                .recent_commits("HEAD", 50)
                .into_iter()
                .map(|commit| CommitDto {
                    hash: commit.hash,
                    short: commit.short,
                    subject: commit.subject,
                    author: commit.author,
                    date: commit.date,
                })
                .collect(),
        }))
    })
    .await
}

async fn diff(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DiffQuery>,
) -> ApiResult<ChangeSet> {
    blocking(move || Ok(Json(load(&state, &query)?.changes.clone()))).await
}

#[derive(Deserialize)]
struct MapQuery {
    /// `architecture`, `package` (the default), `module` or `code`.
    level: Option<String>,
    /// Draw a change: what is there after it, with what was only there
    /// before it kept as ghosts. Without this, the files as they are now.
    #[serde(default)]
    compare: bool,
    spec: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    staged: bool,
}

/// What a map draws at a level: the boxes and the wires among the children
/// of each. A layout in the page turns this into coordinates.
async fn map(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MapQuery>,
) -> ApiResult<mapdiff_graph::MapData> {
    blocking(move || {
        let level = explore::parse_level(query.level.as_deref())?;
        let hidden = std::collections::HashSet::new();
        let which = explore::Which {
            compare: query.compare,
            spec: query.spec,
            commit: query.commit,
            staged: query.staged,
        };
        Ok(Json(match explore::subject(&state, &which)? {
            explore::Subject::Live(live) => live.graph.map(state.repo.root(), level, &hidden),
            explore::Subject::Shown(shown) => {
                shown.compared.map(&shown.loaded.changes, level, &hidden)
            }
        }))
    })
    .await
}

#[derive(Deserialize)]
struct ChangeQuery {
    // The comparison, field by field: nested in a struct of its own, its
    // true and false would not be read from a query string.
    spec: Option<String>,
    commit: Option<String>,
    #[serde(default)]
    staged: bool,
    #[serde(default)]
    w: bool,
    /// Position of the change in the change set.
    index: usize,
    /// The ID the page believes is at that position. When the working tree
    /// changed in between, it no longer is, and the page must reload.
    id: String,
}

async fn change(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ChangeQuery>,
) -> ApiResult<ChangeDetail> {
    blocking(move || {
        let diff = DiffQuery {
            spec: query.spec,
            commit: query.commit,
            staged: query.staged,
            w: query.w,
        };
        let loaded = load(&state, &diff)?;
        let change = loaded
            .changes
            .changes
            .get(query.index)
            .filter(|change| change.id() == query.id)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::CONFLICT,
                    String::from("The files changed since this list was made. Reload it."),
                )
            })?;
        Ok(Json(detail::change_detail(
            &state.repo,
            &loaded.comparison,
            change,
            query.index,
        )))
    })
    .await
}

#[derive(Serialize)]
struct Reviewed {
    keys: Vec<String>,
}

async fn reviewed(State(state): State<Arc<AppState>>) -> ApiResult<Reviewed> {
    blocking(move || {
        Ok(Json(Reviewed {
            keys: state.reviewed.load().into_iter().collect(),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct ReviewedUpdate {
    keys: Vec<String>,
    reviewed: bool,
}

async fn set_reviewed(
    State(state): State<Arc<AppState>>,
    Json(update): Json<ReviewedUpdate>,
) -> ApiResult<Reviewed> {
    blocking(move || {
        let _guard = state.reviewed_lock.lock().unwrap();
        let keys = state
            .reviewed
            .update(&update.keys, update.reviewed)
            .map_err(|err| {
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot save review marks: {err}"),
                )
            })?;
        Ok(Json(Reviewed {
            keys: keys.into_iter().collect(),
        }))
    })
    .await
}

async fn scopes(State(state): State<Arc<AppState>>) -> ApiResult<mapdiff_core::ScopeTree> {
    blocking(move || {
        mapdiff_extract::extract_repo(state.repo.root())
            .map(Json)
            .map_err(|err| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
    })
    .await
}

#[derive(Deserialize)]
struct SourceQuery {
    id: String,
}

#[derive(Serialize)]
struct Source {
    id: String,
    text: String,
}

async fn source(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SourceQuery>,
) -> ApiResult<Source> {
    blocking(move || {
        let root = state.repo.root();
        let tree = mapdiff_extract::extract_repo(root)
            .map_err(|err| ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let scope = tree
            .get(&query.id)
            .filter(|scope| scope.kind != mapdiff_core::ScopeKind::Directory)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::NOT_FOUND,
                    format!("no scope with the ID {:?}", query.id),
                )
            })?;
        let bytes = std::fs::read(root.join(&scope.path.file)).unwrap_or_default();
        let range = scope.range.start_byte as usize..scope.range.end_byte as usize;
        let text = String::from_utf8_lossy(bytes.get(range).unwrap_or(&[])).into_owned();
        Ok(Json(Source { id: query.id, text }))
    })
    .await
}

/// Refuses requests that did not come from a page on this machine.
///
/// The server only listens on loopback, but a web page elsewhere could still
/// make the browser call it. Such a request carries the other site's name in
/// `Origin`, or reaches us through a host name that is not ours.
async fn local_only(headers: HeaderMap, request: Request, next: Next) -> Response {
    let is_local = |value: &str| {
        let host = value
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let name = match host.strip_prefix('[') {
            Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
            None => host.split(':').next().unwrap_or(""),
        };
        matches!(name, "localhost" | "127.0.0.1" | "::1")
    };
    let header_ok = |name: header::HeaderName, required: bool| match headers.get(name) {
        Some(value) => value.to_str().is_ok_and(is_local),
        None => !required,
    };
    if header_ok(header::HOST, true) && header_ok(header::ORIGIN, false) {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "This server only answers pages on this machine.",
        )
            .into_response()
    }
}

const MISSING_PAGE: &str = "<!doctype html><meta charset=utf-8><title>mapdiff</title>\
<body style=\"font-family: system-ui; max-width: 40rem; margin: 4rem auto; padding: 0 1rem\">\
<h1>The page is not built yet</h1>\
<p>The API is running, but there is no built page to serve. Build it once:</p>\
<pre>cd ui &amp;&amp; npm ci &amp;&amp; npm run build</pre>\
<p>Then reload. Or point <code>mapdiff serve --ui-dir</code> at a built page.</p>";

/// The routes, for the real server and for tests.
pub fn router(state: Arc<AppState>, ui_dir: Option<&Path>) -> Router {
    let api = Router::new()
        .route("/meta", get(meta))
        .route("/diff", get(diff))
        .route("/change", get(change))
        .route("/reviewed", get(reviewed).post(set_reviewed))
        .route("/scopes", get(scopes))
        .route("/map", get(map))
        .route("/node", get(explore::node))
        .route("/code", get(explore::code))
        .route("/search", get(explore::search))
        .route("/pr", axum::routing::post(explore::pull_request))
        .route("/source", get(source))
        .route("/running", get(running_now))
        .route("/references", get(indexing::references))
        .route("/index", axum::routing::post(indexing::index))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, String::from("no such API route")) })
        .with_state(state);

    let app = Router::new().nest("/api", api);
    let app = match ui_dir.filter(|directory| directory.join("index.html").is_file()) {
        // A folder was named: serve the page from it, as it is on disk now.
        // The page is one HTML file that routes in the browser, so unknown
        // paths get that file too.
        Some(directory) => app.fallback_service(
            ServeDir::new(directory).fallback(ServeFile::new(directory.join("index.html"))),
        ),
        // Otherwise the page that was built into this program.
        None => app.fallback(built_in_page),
    };
    app.layer(middleware::from_fn(local_only))
}

/// The page, built into the program when it was compiled after the page
/// had been built (`cd ui && npm run build`).
#[cfg(embedded_ui)]
static PAGE: include_dir::Dir<'static> = include_dir::include_dir!("$MAPDIFF_PAGE_DIR");

/// True when this program carries the page inside it.
pub const HAS_PAGE: bool = cfg!(embedded_ui);

#[cfg(embedded_ui)]
async fn built_in_page(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Unknown paths get the page itself, which routes in the browser.
    let (path, file) = match PAGE.get_file(path) {
        Some(file) => (path, file),
        None => match PAGE.get_file("index.html") {
            Some(file) => ("index.html", file),
            None => return Html(MISSING_PAGE).into_response(),
        },
    };
    let kind = match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" | "map" => "application/json",
        "woff2" => "font/woff2",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    };
    // Files under `assets/` have a hash of their content in their name, so
    // the browser may keep them; the page itself is asked for every time.
    let cache = match path.starts_with("assets/") {
        true => "public, max-age=31536000, immutable",
        false => "no-cache",
    };
    (
        [(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache)],
        file.contents(),
    )
        .into_response()
}

#[cfg(not(embedded_ui))]
async fn built_in_page() -> Response {
    Html(MISSING_PAGE).into_response()
}

/// A folder to serve the page from instead of the one built in: for
/// working on the page.
pub fn ui_dir_override() -> Option<PathBuf> {
    std::env::var_os("MAPDIFF_UI_DIR").map(PathBuf::from)
}

/// Which process this is and which repository it reads, so that a note of
/// a running server can be checked against what answers on its port.
async fn running_now(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "pid": std::process::id(),
        "repo": state.repo.root().to_string_lossy(),
    }))
}

/// Ends when the server is asked to stop: Ctrl-C, or the signal `kill`
/// sends by default.
async fn asked_to_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// Starts the server and blocks until it is stopped with Ctrl-C or by
/// `mapdiff stop`. `ready` is called with the address once it is listening.
pub fn serve(options: ServerOptions, ready: impl FnOnce(&str)) -> Result<(), String> {
    let mut state = AppState::new(&options.repo)?;
    state.auto_index = options.index;
    let state = Arc::new(state);
    let ui_dir = options.ui_dir.or_else(ui_dir_override);
    let app = router(state.clone(), ui_dir.as_deref());
    let home = mapdiff_graph::cache_home();

    let runtime = tokio::runtime::Runtime::new().map_err(|err| err.to_string())?;
    runtime.block_on(async move {
        let mut listener = None;
        for port in options.port..options.port.saturating_add(20) {
            // Loopback only: nothing outside this machine can connect.
            if let Ok(bound) =
                tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await
            {
                listener = Some(bound);
                break;
            }
        }
        let listener = listener.ok_or_else(|| format!("no free port from {} up", options.port))?;
        let address = listener.local_addr().map_err(|err| err.to_string())?;

        let running = running::Running::new(address.port(), state.repo.root(), &options.showing);
        running::announce(&home, &running);
        running::remember_repository(&home, state.repo.root());
        *state.announced.lock().unwrap() = Some(running);
        if options.index {
            indexing::start(&state);
        }
        ready(&format!("http://127.0.0.1:{}", address.port()));

        let served = axum::serve(listener, app)
            .with_graceful_shutdown(asked_to_stop())
            .await
            .map_err(|err| err.to_string());

        running::withdraw(&home, std::process::id());
        // A running indexer keeps the state alive past this point, so the
        // temporary copy of a comparison is let go of here.
        *state.shown.lock().unwrap() = None;
        served
    })
}
