//! CORS allowlist behaviour for the dashboard server.
//!
//! `cors_for(Some(addr))` (in `lib.rs`) builds an allowlist of
//! `http://127.0.0.1:<port>` and `http://localhost:<port>` for loopback
//! binds. We exercise that by:
//!
//!   1. binding to `127.0.0.1:0`,
//!   2. extracting the OS-assigned port,
//!   3. firing a CORS preflight (`OPTIONS` + `Origin` + `ACR-Method`),
//!   4. asserting the response's `Access-Control-Allow-Origin` header.
//!
//! The fixture port number that ships with most Vite dev servers (5173)
//! is incidental — the allowlist is keyed off the *bound* port. That's
//! the behaviour `lib.rs:50-71` documents and that's what we lock in
//! here. We feed `5173` through the URL only to keep the test names
//! intuitive for readers grepping for the CORS test names.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use ua_core::{KnowledgeGraph, ProjectMeta};
use ua_server::{router_for, AppState};

fn empty_graph() -> KnowledgeGraph {
    KnowledgeGraph::new(ProjectMeta {
        name: "cors-fixture".into(),
        languages: vec![],
        frameworks: vec![],
        description: String::new(),
        analyzed_at: String::new(),
        git_commit_hash: String::new(),
    })
}

/// Boot a real server, return base URL + bound port + the shutdown
/// handle. The port is what the loopback allowlist gets keyed on.
async fn boot() -> (String, u16, tokio::task::JoinHandle<()>) {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().unwrap();
    let port = bound.port();
    let state = AppState::with_graphs(empty_graph(), None, None, PathBuf::new());
    let app = router_for(Arc::new(state), bound);
    let base = format!("http://{bound}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, port, handle)
}

/// Send a CORS preflight (`OPTIONS` + `Origin` + `Access-Control-Request-Method`)
/// against `{base}/api/health` and return the response.
async fn preflight(base: &str, origin: &str) -> reqwest::Response {
    reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("{base}/api/health"))
        .header("Origin", origin)
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("preflight request")
}

#[tokio::test]
async fn test_cors_allows_loopback_origin() {
    let (base, port, handle) = boot().await;
    let origin = format!("http://127.0.0.1:{port}");

    let resp = preflight(&base, &origin).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert_eq!(
        allow.as_deref(),
        Some(origin.as_str()),
        "expected ACAO to echo the loopback origin"
    );

    handle.abort();
}

#[tokio::test]
async fn test_cors_allows_localhost_alias() {
    // The allowlist is `[127.0.0.1:<port>, localhost:<port>]` — feeding
    // `localhost` should produce the same echo.
    let (base, port, handle) = boot().await;
    let origin = format!("http://localhost:{port}");

    let resp = preflight(&base, &origin).await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert_eq!(
        allow.as_deref(),
        Some(origin.as_str()),
        "expected ACAO to echo the localhost alias"
    );

    handle.abort();
}

#[tokio::test]
async fn test_cors_rejects_foreign_origin() {
    // Anything outside the loopback dual-host pair must NOT be
    // reflected back. tower-http's `CorsLayer` simply omits the
    // `Access-Control-Allow-Origin` header in that case (the request
    // itself still returns a non-error status — the *browser* enforces
    // the ban, the server just refuses to bless it).
    let (base, _port, handle) = boot().await;

    let resp = preflight(&base, "https://evil.example.com").await;
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert!(
        allow.as_deref() != Some("https://evil.example.com"),
        "foreign origin must not be echoed back, got {allow:?}"
    );

    handle.abort();
}
