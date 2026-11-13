//! Running the language indexers from the server, in the background, and
//! telling the page how references are found right now.
//!
//! The map is drawn from names straight away. An index adds what only
//! types can tell, and can take minutes to make, so it is made on another
//! thread and the page asks how far along it is.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Query, State};
use mapdiff_graph::{Outcome, Precise, TierReport};
use serde::Serialize;

use crate::explore::{Live, Which, subject};
use crate::{ApiResult, AppState, blocking};

/// What the server knows about making indexes.
#[derive(Default)]
pub struct Indexing {
    running: Option<Run>,
    /// Why the last run could not index a project: a missing tool, or one
    /// that failed.
    notes: Vec<String>,
    /// The files as they were when the last run ended, so that one is not
    /// started again for files it has already been tried on.
    attempted: Option<u64>,
}

struct Run {
    started: Instant,
    step: String,
}

/// Starts the indexers on another thread, unless they are running. Says
/// whether it started them.
pub fn start(state: &Arc<AppState>) -> bool {
    {
        let mut indexing = state.indexing.lock().unwrap();
        if indexing.running.is_some() {
            return false;
        }
        indexing.running = Some(Run {
            started: Instant::now(),
            step: String::from("reading the files"),
        });
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let root = state.repo.root().to_path_buf();
        let before = mapdiff_extract::stamp(&root).ok();
        let built = mapdiff_graph::build_with(&root, Precise::Refresh, &|step| {
            if let Some(run) = state.indexing.lock().unwrap().running.as_mut() {
                run.step = step.to_owned();
            }
        });
        let after = mapdiff_extract::stamp(&root).ok();
        let notes = match &built {
            Ok(graph) => notes_of(&graph.tiers),
            Err(err) => vec![format!("indexing failed: {err}")],
        };
        // The graph is of the files as they were when the run began. It
        // is kept only when none changed meanwhile; otherwise the next
        // request reads them again.
        if let Ok(graph) = built
            && let Some(stamp) = after
            && before == after
        {
            *state.live.lock().unwrap() = Some(Arc::new(Live::new(stamp, graph)));
        }
        // A comparison was built before there was an index.
        *state.shown.lock().unwrap() = None;
        let mut indexing = state.indexing.lock().unwrap();
        indexing.running = None;
        indexing.notes = notes;
        indexing.attempted = after;
    });
    true
}

fn notes_of(tiers: &[TierReport]) -> Vec<String> {
    tiers
        .iter()
        .filter_map(|tier| {
            let place = match tier.directory.as_str() {
                "." => String::new(),
                directory => format!(" (in {directory})"),
            };
            match &tier.outcome {
                Outcome::ToolMissing { install } => Some(format!(
                    "{} is not installed{place}. To get it: {install}",
                    tier.tool
                )),
                Outcome::Failed(message) => Some(format!("{message}{place}")),
                Outcome::Used { .. } | Outcome::NotIndexed => None,
            }
        })
        .collect()
}

/// How references are found for what the page is showing.
#[derive(Serialize)]
pub struct References {
    /// `name`: from names and imports only. `type`: every project also has
    /// an index. `mixed`: some projects have one.
    by: &'static str,
    /// Whether making an index could add something: the page offers it.
    can_index: bool,
    /// Set while the indexers are running.
    indexing: Option<Progress>,
    /// Why the last run left a project without an index.
    notes: Vec<String>,
}

#[derive(Serialize)]
pub struct Progress {
    step: String,
    seconds: u64,
}

pub async fn references(
    State(state): State<Arc<AppState>>,
    Query(which): Query<Which>,
) -> ApiResult<References> {
    blocking(move || {
        let subject = subject(&state, &which)?;
        let (graph, root) = subject.head(&state);
        let projects = graph.tiers.len();
        let indexed = graph
            .tiers
            .iter()
            .filter(|tier| matches!(tier.outcome, Outcome::Used { .. }))
            .count();
        // An index covers the files as they are now. A commit that was
        // written out to be read is found by name only.
        let working_tree = root == state.repo.root();
        let incomplete = working_tree && indexed < projects;

        if state.auto_index && incomplete {
            let stamp = mapdiff_extract::stamp(state.repo.root()).ok();
            let attempted = state.indexing.lock().unwrap().attempted;
            if stamp.is_some() && stamp != attempted {
                start(&state);
            }
        }

        let indexing = state.indexing.lock().unwrap();
        let progress = indexing.running.as_ref().map(|run| Progress {
            step: run.step.clone(),
            seconds: run.started.elapsed().as_secs(),
        });
        Ok(Json(References {
            by: if projects > 0 && indexed == projects {
                "type"
            } else if indexed == 0 {
                "name"
            } else {
                "mixed"
            },
            can_index: incomplete && progress.is_none(),
            notes: if working_tree {
                indexing.notes.clone()
            } else {
                Vec::new()
            },
            indexing: progress,
        }))
    })
    .await
}

#[derive(Serialize)]
pub struct Started {
    /// False when the indexers were already running.
    started: bool,
}

pub async fn index(State(state): State<Arc<AppState>>) -> ApiResult<Started> {
    Ok(Json(Started {
        started: start(&state),
    }))
}
