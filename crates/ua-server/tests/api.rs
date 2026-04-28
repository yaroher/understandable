//! End-to-end exercise of the dashboard JSON API.
//!
//! We boot a real `axum::serve` on a random ephemeral port against a
//! handcrafted `AppState` and hit the routes with `reqwest`, asserting
//! the documented behaviour for each fix:
//!
//!   * `/api/graph` honours `?kind=…` and 404s on missing kinds.
//!   * `/api/diff` returns the storage-dir overlay file as JSON, or
//!     `204 No Content` when none is present.
//!   * Pagination tolerates `offset` values close to `usize::MAX`
//!     without overflowing.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use ua_core::{KnowledgeGraph, ProjectMeta};
use ua_server::{router_for, AppState};

fn empty_graph(project_name: &str) -> KnowledgeGraph {
    KnowledgeGraph::new(ProjectMeta {
        name: project_name.into(),
        languages: vec![],
        frameworks: vec![],
        description: String::new(),
        analyzed_at: String::new(),
        git_commit_hash: String::new(),
    })
}

/// Spin up the server in a background task and return its base URL plus
/// the `tempfile::TempDir` so the storage directory keeps existing for
/// the lifetime of the test.
async fn boot_with(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().unwrap();
    let app = router_for(Arc::new(state), bound);
    let base = format!("http://{bound}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, handle)
}

#[tokio::test]
async fn graph_endpoint_returns_codebase_by_default() {
    let state = AppState::with_graphs(
        empty_graph("codebase-only"),
        None,
        None,
        PathBuf::new(),
    );
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/graph"))
        .await
        .expect("request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["project"]["name"], "codebase-only");

    handle.abort();
}

#[tokio::test]
async fn graph_endpoint_honours_kind_param() {
    let state = AppState::with_graphs(
        empty_graph("code"),
        Some(empty_graph("domain-overlay")),
        None,
        PathBuf::new(),
    );
    let (base, handle) = boot_with(state).await;

    // domain is loaded -> 200 with the matching project name.
    let resp = reqwest::get(format!("{base}/api/graph?kind=domain"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["project"]["name"], "domain-overlay");

    // knowledge is *not* loaded -> 404 with a descriptive body.
    let resp = reqwest::get(format!("{base}/api/graph?kind=knowledge"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("knowledge"),
        "expected 404 body to mention the missing kind, got: {text}"
    );

    // Garbage kind -> 400.
    let resp = reqwest::get(format!("{base}/api/graph?kind=bogus"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
}

#[tokio::test]
async fn diff_endpoint_returns_no_content_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_graphs(
        empty_graph("p"),
        None,
        None,
        dir.path().to_path_buf(),
    );
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/diff")).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);

    handle.abort();
    drop(dir);
}

#[tokio::test]
async fn diff_endpoint_returns_overlay_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let overlay = json!({
        "changedNodeIds": ["a", "b"],
        "affectedNodeIds": ["c"],
    });
    std::fs::write(
        dir.path().join("diff-overlay.json"),
        serde_json::to_vec(&overlay).unwrap(),
    )
    .unwrap();

    let state = AppState::with_graphs(
        empty_graph("p"),
        None,
        None,
        dir.path().to_path_buf(),
    );
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/diff")).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, overlay);

    handle.abort();
    drop(dir);
}

#[tokio::test]
async fn pagination_does_not_overflow_on_huge_offset() {
    let state = AppState::with_graphs(
        empty_graph("p"),
        None,
        None,
        PathBuf::new(),
    );
    let (base, handle) = boot_with(state).await;

    // `offset + limit` would wrap a `usize` before the
    // `saturating_add` fix was applied. We expect a clean 200 with an
    // empty page.
    let url = format!(
        "{base}/api/graph/nodes?offset={}&limit={}",
        usize::MAX - 1,
        1000
    );
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["limit"], 1000);

    // Same guard for edges.
    let url = format!(
        "{base}/api/graph/edges?offset={}&limit={}",
        usize::MAX,
        1
    );
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    handle.abort();
}
