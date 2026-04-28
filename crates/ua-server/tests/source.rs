//! Integration tests for the `/api/source` endpoint.
//!
//! Boots a real `axum::serve` against a temp project root, writes a few
//! files, then hits the route via `reqwest`. Covers the three things
//! the dashboard depends on:
//!
//!   1. Slicing returns exactly the requested line range.
//!   2. `..` traversal attempts are refused with `400`.
//!   3. Files that escape the project root after join are also `400`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use ua_core::{KnowledgeGraph, ProjectMeta};
use ua_server::{router_for, AppState};

fn empty_graph() -> KnowledgeGraph {
    KnowledgeGraph::new(ProjectMeta {
        name: "src-test".into(),
        languages: vec![],
        frameworks: vec![],
        description: String::new(),
        analyzed_at: String::new(),
        git_commit_hash: String::new(),
    })
}

async fn boot(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
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
async fn test_source_returns_slice() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    // 5 lines of trivial content; we'll request lines 2..=4.
    let file = root.join("foo.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\ndelta\nepsilon\n").unwrap();

    let state =
        AppState::with_graphs_and_root(empty_graph(), None, None, PathBuf::new(), root.clone());
    let (base, handle) = boot(state).await;

    let url = format!("{base}/api/source?path=foo.txt&start=2&end=4");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/plain"),
        "unexpected content-type: {ct}"
    );
    let body = resp.text().await.unwrap();
    assert_eq!(body, "beta\ngamma\ndelta");

    // No range -> full file.
    let url = format!("{base}/api/source?path=foo.txt");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("alpha"));
    assert!(body.contains("epsilon"));

    handle.abort();
    drop(dir);
}

#[tokio::test]
async fn test_source_rejects_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    // A file *outside* the project root that we definitely don't want
    // anyone to read via the API.
    let outside_dir = tempfile::tempdir().unwrap();
    std::fs::write(outside_dir.path().join("secret.txt"), "shh\n").unwrap();

    let state =
        AppState::with_graphs_and_root(empty_graph(), None, None, PathBuf::new(), root.clone());
    let (base, handle) = boot(state).await;

    // 1. Direct `..` traversal — rejected up front.
    let url = format!("{base}/api/source?path=../../etc/passwd");
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("..") || body.contains("rejected"),
        "expected traversal rejection text, got: {body}"
    );

    // 2. Absolute path that points outside the root — the
    //    canonicalised target won't start with our root prefix.
    let outside_abs = outside_dir.path().join("secret.txt");
    let url = format!(
        "{base}/api/source?path={}",
        urlencoding(outside_abs.to_string_lossy().as_ref())
    );
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. Empty path — also a 400 so callers can't trick the API into a
    //    no-op read of the project root itself.
    let resp = reqwest::get(format!("{base}/api/source?path="))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    handle.abort();
    drop(dir);
    drop(outside_dir);
}

/// Tiny URL encoder for path values that may contain `/`. We can't
/// pull in `urlencoding` as a dev-dep without disturbing other agents'
/// Cargo.toml work, so this is a hand-rolled minimal version that
/// encodes the few bytes that would otherwise break the query.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
