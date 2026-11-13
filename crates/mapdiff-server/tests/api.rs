//! The HTTP API, called directly without a network socket.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use mapdiff_server::{AppState, router};
use mapdiff_testkit::TempRepo;
use serde_json::{Value, json};
use tower::ServiceExt;

const BASE: &str = "\
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn double(value: i32) -> i32 {
    add(value, value)
}
";

/// A repository with two commits: the second edits `add`.
fn repo(name: &str) -> TempRepo {
    let repo = TempRepo::new(name);
    repo.write("src/lib.rs", BASE);
    repo.commit("base");
    repo.replace("src/lib.rs", "    a + b\n", "    a - b\n");
    repo.commit("subtract instead");
    repo
}

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&self.body)))
    }
}

async fn send(repo: &TempRepo, request: Request<Body>) -> Reply {
    let state = Arc::new(AppState::new(repo.path()).expect("state builds"));
    let response = router(state, None)
        .oneshot(request)
        .await
        .expect("router answers");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable")
        .to_vec();
    Reply { status, body }
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .header(header::HOST, "127.0.0.1:7878")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn meta_describes_the_repository() {
    let repo = repo("meta");
    let reply = send(&repo, get("/api/meta")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let meta = reply.json();
    assert_eq!(meta["branch"], "main");
    assert_eq!(meta["default_branch"], "main");
    assert_eq!(meta["commits"].as_array().unwrap().len(), 2);
    assert_eq!(meta["commits"][0]["subject"], "subtract instead");
}

#[tokio::test]
async fn diff_returns_the_change_set_of_a_commit() {
    let repo = repo("diff");
    let reply = send(&repo, get("/api/diff?commit=HEAD")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let changes = reply.json();
    assert_eq!(changes["stats"]["scopes"], 1);
    assert_eq!(changes["stats"]["added"], 1);
    let ids: Vec<&str> = changes["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|change| change["new_id"].as_str())
        .collect();
    assert!(ids.contains(&"src/lib.rs::add()"), "{ids:?}");
    assert!(!ids.contains(&"src/lib.rs::double()"), "{ids:?}");
}

#[tokio::test]
async fn a_clean_working_tree_is_an_empty_change_set() {
    let repo = repo("clean");
    let changes = send(&repo, get("/api/diff")).await.json();
    assert_eq!(changes["stats"]["files"], 0);
    assert_eq!(changes["head"]["label"], "working tree");
}

#[tokio::test]
async fn change_returns_highlighted_rows_with_the_changed_words_marked() {
    let repo = repo("change");
    let changes = send(&repo, get("/api/diff?commit=HEAD")).await.json();
    let index = changes["changes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|change| change["new_id"] == "src/lib.rs::add()")
        .unwrap();

    let path = format!("/api/change?commit=HEAD&index={index}&id=src/lib.rs::add()");
    let reply = send(&repo, get(&path)).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let detail = reply.json();
    assert_eq!(detail["old"]["first_line"], 1);
    assert_eq!(detail["new"]["last_line"], 3);

    let rows = detail["rows"].as_array().unwrap();
    let tags: Vec<&str> = rows
        .iter()
        .map(|row| row["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["equal", "delete", "insert", "equal"]);

    // The first row is `pub fn add(...)`: `pub` and `fn` are keywords.
    let keywords: Vec<&str> = rows[0]["spans"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|span| span["class"] == "keyword")
        .map(|span| span["text"].as_str().unwrap())
        .collect();
    assert_eq!(keywords, ["pub", "fn"]);

    // On the inserted line only the operator differs from the deleted one.
    let changed: Vec<&str> = rows[2]["spans"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|span| span["changed"] == true)
        .map(|span| span["text"].as_str().unwrap())
        .collect();
    assert_eq!(changed, ["-"]);
    assert_eq!(rows[1]["old_line"], 2);
    assert_eq!(rows[1]["new_line"], Value::Null);
    assert_eq!(rows[2]["new_line"], 2);
}

#[tokio::test]
async fn a_stale_position_is_a_conflict_not_the_wrong_scope() {
    let repo = repo("stale");
    let reply = send(
        &repo,
        get("/api/change?commit=HEAD&index=1&id=some/other.rs::thing()"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(reply.json()["error"].as_str().unwrap().contains("Reload"));
}

#[tokio::test]
async fn a_bad_revision_is_a_readable_error() {
    let repo = repo("bad");
    let reply = send(&repo, get("/api/diff?spec=no-such-branch")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(
        reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("no-such-branch")
    );
}

#[tokio::test]
async fn review_marks_are_saved_inside_the_git_directory() {
    let repo = repo("reviewed");
    assert_eq!(
        send(&repo, get("/api/reviewed")).await.json()["keys"],
        json!([])
    );

    let post = |body: Value| {
        Request::post("/api/reviewed")
            .header(header::HOST, "localhost:7878")
            .header(header::ORIGIN, "http://localhost:7878")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let reply = send(&repo, post(json!({ "keys": ["b", "a"], "reviewed": true }))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["keys"], json!(["a", "b"]));

    // `send` builds a fresh server each time, so this read comes from disk.
    assert_eq!(
        send(&repo, get("/api/reviewed")).await.json()["keys"],
        json!(["a", "b"])
    );
    assert!(repo.path().join(".git/mapdiff/reviewed.json").is_file());

    let reply = send(&repo, post(json!({ "keys": ["a"], "reviewed": false }))).await;
    assert_eq!(reply.json()["keys"], json!(["b"]));

    // The marks file is not something git sees.
    assert_eq!(repo.git(&["status", "--porcelain"]), "");
}

#[tokio::test]
async fn requests_from_other_sites_are_refused() {
    let repo = repo("local-only");

    // Reached through a host name that is not this machine's.
    let foreign_host = Request::get("/api/meta")
        .header(header::HOST, "evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&repo, foreign_host).await.status,
        StatusCode::FORBIDDEN
    );

    // Sent by a page on another site.
    let foreign_origin = Request::post("/api/reviewed")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "keys": ["x"], "reviewed": true }).to_string(),
        ))
        .unwrap();
    assert_eq!(
        send(&repo, foreign_origin).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&repo, get("/api/reviewed")).await.json()["keys"],
        json!([])
    );

    // A name that only starts like ours.
    let lookalike = Request::get("/api/meta")
        .header(header::HOST, "localhost.evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&repo, lookalike).await.status, StatusCode::FORBIDDEN);

    let no_host = Request::get("/api/meta").body(Body::empty()).unwrap();
    assert_eq!(send(&repo, no_host).await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn scopes_and_source_serve_the_working_tree() {
    let repo = repo("scopes");
    let tree = send(&repo, get("/api/scopes")).await.json();
    let ids: Vec<&str> = tree["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|scope| scope["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"src/lib.rs::double()"), "{ids:?}");

    let source = send(&repo, get("/api/source?id=src/lib.rs::add()"))
        .await
        .json();
    assert!(source["text"].as_str().unwrap().contains("a - b"));

    let missing = send(&repo, get("/api/source?id=src/lib.rs::nope()")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_api_routes_are_explained() {
    let repo = repo("fallbacks");
    let reply = send(&repo, get("/api/nothing")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert!(reply.json()["error"].is_string());
}

#[tokio::test]
async fn map_gives_boxes_and_wires_at_a_level() {
    let repo = repo("map");
    let reply = send(&repo, get("/api/map?level=module")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let map = reply.json();
    assert_eq!(map["level"], "module");
    let ids: Vec<&str> = map["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            ".",
            "src",
            "src/lib.rs",
            "src/lib.rs::add()",
            "src/lib.rs::double()"
        ]
    );
    let wire = &map["containers"]["src/lib.rs"]["wires"][0];
    assert_eq!(wire["from"], "src/lib.rs::double()");
    assert_eq!(wire["to"], "src/lib.rs::add()");
    assert_eq!(wire["presence"], "both");

    let reply = send(&repo, get("/api/map?level=galaxy")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn map_of_a_change_keeps_what_was_removed() {
    let repo = repo("map-compare");
    // Uncommitted: `double` is deleted.
    let kept = BASE
        .split("pub fn double")
        .next()
        .unwrap()
        .replace("a + b", "a - b");
    repo.write("src/lib.rs", &kept);

    let reply = send(&repo, get("/api/map?level=module&compare=true")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let map = reply.json();
    let ghost = map["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "src/lib.rs::double()")
        .expect("the removed function is still on the map");
    assert_eq!(ghost["presence"], "removed");
    assert_eq!(
        map["containers"]["src/lib.rs"]["wires"][0]["presence"],
        "removed"
    );

    // Without `compare`, the map is of the files as they are.
    let reply = send(&repo, get("/api/map?level=module")).await;
    assert_eq!(reply.json()["nodes"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn node_lists_who_uses_a_scope_and_what_it_uses() {
    let repo = repo("node");
    let reply = send(&repo, get("/api/node?id=src/lib.rs::add()&level=module")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let node = reply.json();
    assert_eq!(node["label"], "add");
    assert_eq!(node["path"], "src/lib.rs");
    assert_eq!(
        (node["first_line"].as_u64(), node["last_line"].as_u64()),
        (Some(1), Some(3))
    );
    assert_eq!(node["signature"], "pub fn add(a: i32, b: i32) -> i32");
    let crumbs: Vec<&str> = node["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|crumb| crumb["id"].as_str().unwrap())
        .collect();
    assert_eq!(crumbs, [".", "src", "src/lib.rs"]);

    let caller = &node["callers"][0];
    assert_eq!(caller["id"], "src/lib.rs::double()");
    assert_eq!(caller["at"], "src/lib.rs::double()");
    assert_eq!(caller["count"], 1);
    // Where: line 6, at the name `add`, with the line as written.
    let place = &caller["places"][0];
    assert_eq!(place["line"], 6);
    assert_eq!(place["text"], "add(value, value)");
    assert_eq!(
        (place["indent"].as_u64(), place["column"].as_u64()),
        (Some(4), Some(4))
    );
    assert_eq!(place["end_column"], 7);
    assert!(node["dependencies"].as_array().unwrap().is_empty());

    // At a higher level the caller is drawn as its file.
    let reply = send(
        &repo,
        get("/api/node?id=src/lib.rs::double()&level=package"),
    )
    .await;
    let node = reply.json();
    assert_eq!(node["dependencies"][0]["id"], "src/lib.rs::add()");
    assert_eq!(node["dependencies"][0]["at"], "src/lib.rs");

    let reply = send(&repo, get("/api/node?id=nowhere.rs")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn code_gives_a_file_as_highlighted_lines() {
    let repo = repo("code");
    let reply = send(&repo, get("/api/code?path=src/lib.rs")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let code = reply.json();
    let lines = code["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 7);
    let first: String = lines[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| span["text"].as_str().unwrap())
        .collect();
    assert_eq!(first, "pub fn add(a: i32, b: i32) -> i32 {");
    assert_eq!(lines[0][0]["class"], "keyword");

    // Only files of the repository, by the name the tree knows them by.
    for path in ["../secret", "/etc/passwd", "src", "src/missing.rs"] {
        let reply = send(&repo, get(&format!("/api/code?path={path}"))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn search_finds_scopes_by_a_few_letters() {
    let repo = repo("search");
    let reply = send(&repo, get("/api/search?q=dbl")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let hits = reply.json();
    assert_eq!(hits["hits"][0]["id"], "src/lib.rs::double()");
    assert_eq!(hits["hits"][0]["level"], "module");

    let hits = send(&repo, get("/api/search?q=lib")).await.json();
    assert_eq!(hits["hits"][0]["id"], "src/lib.rs");
    assert_eq!(hits["hits"][0]["level"], "package");

    let hits = send(&repo, get("/api/search?q=zzzz")).await.json();
    assert!(hits["hits"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_graph_is_kept_until_a_file_changes() {
    let repo = repo("live");
    let state = Arc::new(AppState::new(repo.path()).expect("state builds"));
    let ask = |state: Arc<AppState>| async move {
        let response = router(state, None)
            .oneshot(get("/api/search?q=triple"))
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice::<Value>(&body).unwrap()["hits"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(ask(state.clone()).await, 0);
    repo.write(
        "src/lib.rs",
        &format!(
            "{BASE}\npub fn triple(value: i32) -> i32 {{\n    add(value, double(value))\n}}\n"
        ),
    );
    assert_eq!(ask(state.clone()).await, 1);
}

#[tokio::test]
async fn the_map_of_a_change_says_which_boxes_changed() {
    let repo = repo("overlay");
    // The second commit edited the body of `add`.
    let reply = send(&repo, get("/api/map?level=module&compare=true&commit=HEAD")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let map = reply.json();
    let overlay = &map["overlay"];
    assert_eq!(overlay["changes"]["src/lib.rs::add()"], json!([3]));
    assert_eq!(overlay["order"], json!([3]));
    // `double` uses `add` and did not change. A new body is not something
    // it has to care about.
    assert_eq!(overlay["near"], json!(["src/lib.rs::double()"]));
    assert_eq!(overlay["impacted"], json!({}));

    // Without a comparison there is no overlay.
    let plain = send(&repo, get("/api/map?level=module")).await.json();
    assert_eq!(plain["overlay"], Value::Null);
}

#[tokio::test]
async fn a_comparison_answers_for_both_of_its_sides() {
    let repo = repo("sides");
    // Uncommitted: `double` is deleted.
    let kept = BASE
        .split("pub fn double")
        .next()
        .unwrap()
        .replace("a + b", "a - b");
    repo.write("src/lib.rs", &kept);

    // The removed function is still known, from the side before.
    let reply = send(
        &repo,
        get("/api/node?id=src/lib.rs::double()&level=module&compare=true"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let node = reply.json();
    assert_eq!(node["side"], "base");
    assert_eq!(node["dependencies"][0]["id"], "src/lib.rs::add()");
    // Without the comparison it is not there.
    let reply = send(&repo, get("/api/node?id=src/lib.rs::double()&level=module")).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    // The file as it was, and as it is.
    let lines = |reply: Reply| reply.json()["lines"].as_array().unwrap().len();
    let before = send(
        &repo,
        get("/api/code?path=src/lib.rs&compare=true&side=base"),
    )
    .await;
    let after = send(&repo, get("/api/code?path=src/lib.rs&compare=true")).await;
    assert_eq!((lines(before), lines(after)), (7, 4));
    let reply = send(&repo, get("/api/code?path=src/lib.rs&side=base")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_commit_is_drawn_from_its_own_files_not_the_working_tree() {
    let repo = repo("commit-sides");
    // The working tree moves on after the commit that is looked at.
    repo.write("src/lib.rs", "pub fn other() {}\n");

    let reply = send(
        &repo,
        get("/api/code?path=src/lib.rs&compare=true&commit=HEAD"),
    )
    .await;
    let first: String = reply.json()["lines"][1]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| span["text"].as_str().unwrap())
        .collect();
    assert_eq!(first, "    a - b");
    let node = send(
        &repo,
        get("/api/node?id=src/lib.rs::add()&level=module&compare=true&commit=HEAD"),
    )
    .await
    .json();
    assert_eq!(
        (node["side"].as_str(), node["callers"][0]["id"].as_str()),
        (Some("head"), Some("src/lib.rs::double()"))
    );
}

#[tokio::test]
async fn the_before_and_after_of_a_staged_change_can_be_asked_for() {
    // The comparison is part of this query next to other fields; it must
    // be read the same way as on its own.
    let repo = repo("staged-detail");
    repo.replace("src/lib.rs", "    a - b\n", "    a * b\n");
    repo.git(&["add", "src/lib.rs"]);
    let diff = send(&repo, get("/api/diff?staged=true")).await.json();
    let index = diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|change| change["new_id"] == "src/lib.rs::add()")
        .unwrap();
    let reply = send(
        &repo,
        get(&format!(
            "/api/change?staged=true&index={index}&id=src/lib.rs::add()"
        )),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
}

#[tokio::test]
async fn the_page_is_served_from_inside_the_program() {
    let repo = repo("page");
    let reply = send(&repo, get("/")).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = String::from_utf8_lossy(&reply.body).into_owned();
    if !mapdiff_server::HAS_PAGE {
        // Built without the page: the server says how to build it.
        assert!(body.contains("The page is not built yet"), "{body}");
        return;
    }
    assert!(body.contains("<div id=\"root\">"), "{body}");
    // A script the page names is there too, and an unknown path gets the
    // page, which routes in the browser.
    let script = body
        .split("src=\"/")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the page loads a script");
    let reply = send(&repo, get(&format!("/{script}"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.len() > 10_000);
    let reply = send(&repo, get("/some/where")).await;
    assert!(String::from_utf8_lossy(&reply.body).contains("<div id=\"root\">"));
}
