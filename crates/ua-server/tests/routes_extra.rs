//! Integration coverage for the JSON API routes that the original
//! `tests/api.rs` left untested:
//!
//!   * `/api/health`  — version sniff.
//!   * `/api/project` — project metadata mirror.
//!   * `/api/layers`  — layer list (with `?kind=` honour).
//!   * `/api/tour`    — tour list (with `?kind=` honour).
//!   * `/api/search`  — search hits (with `?type=` filter).
//!   * `/api/node`    — single-node lookup + 404.
//!   * `/api/neighbors` — neighbour subgraph + 404.
//!
//! We boot a real `axum::serve` on a random ephemeral port the same way
//! `api.rs` does, and assert against the JSON bodies via `reqwest`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphNode, KnowledgeGraph, Layer, NodeType,
    ProjectMeta, TourStep,
};
use ua_server::{router_for, AppState};

/// Build an empty graph carrying just a `ProjectMeta`.
fn empty_graph(project_name: &str) -> KnowledgeGraph {
    KnowledgeGraph::new(ProjectMeta {
        name: project_name.into(),
        languages: vec!["rust".into()],
        frameworks: vec!["axum".into()],
        description: "fixture".into(),
        analyzed_at: "2026-01-01T00:00:00Z".into(),
        git_commit_hash: "deadbeef".into(),
    })
}

/// Build a small graph with two nodes joined by one edge plus a layer
/// and a tour step. Just enough to exercise the various list endpoints.
fn populated_graph(project_name: &str) -> KnowledgeGraph {
    let mut g = empty_graph(project_name);
    g.nodes.push(GraphNode {
        id: "alpha".into(),
        node_type: NodeType::Function,
        name: "alpha_fn".into(),
        file_path: Some("src/alpha.rs".into()),
        line_range: Some((1, 10)),
        summary: "the alpha test function".into(),
        tags: vec!["test".into(), "core".into()],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    });
    g.nodes.push(GraphNode {
        id: "beta".into(),
        node_type: NodeType::Class,
        name: "BetaThing".into(),
        file_path: Some("src/beta.rs".into()),
        line_range: Some((20, 40)),
        summary: "a class that holds state".into(),
        tags: vec!["model".into()],
        complexity: Complexity::Moderate,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    });
    // alpha calls beta — also gives the neighbours endpoint something
    // to chew on.
    g.edges.push(GraphEdge {
        source: "alpha".into(),
        target: "beta".into(),
        edge_type: EdgeType::Calls,
        direction: EdgeDirection::Forward,
        description: Some("alpha invokes Beta::ctor".into()),
        weight: 1.0,
    });
    g.layers.push(Layer {
        id: "service".into(),
        name: "Service".into(),
        description: "service layer".into(),
        node_ids: vec!["alpha".into(), "beta".into()],
    });
    g.tour.push(TourStep {
        order: 1,
        title: "Start here".into(),
        description: "Walk through alpha then beta".into(),
        node_ids: vec!["alpha".into(), "beta".into()],
        language_lesson: None,
    });
    g
}

/// Spin up `axum::serve` against `state` on `127.0.0.1:0` and return
/// the base URL plus the join handle. Caller is expected to `abort()`
/// at the end of the test.
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
async fn test_health_returns_version() {
    let state = AppState::with_graphs(empty_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/health")).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();

    // The route reports its own crate version — pull it from
    // `CARGO_PKG_VERSION` so this test floats with the workspace.
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["status"], "ok");

    handle.abort();
}

#[tokio::test]
async fn test_project_returns_meta() {
    let state = AppState::with_graphs(
        empty_graph("fixture-project"),
        None,
        None,
        PathBuf::new(),
    );
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/project"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "fixture-project");
    assert_eq!(body["languages"][0], "rust");
    assert_eq!(body["frameworks"][0], "axum");

    handle.abort();
}

#[tokio::test]
async fn test_layers_returns_array() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/layers"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("layers must be a JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "service");
    assert_eq!(arr[0]["name"], "Service");
    assert_eq!(arr[0]["nodeIds"].as_array().unwrap().len(), 2);

    handle.abort();
}

#[tokio::test]
async fn test_tour_returns_array() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/tour")).await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("tour must be a JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["order"], 1);
    assert_eq!(arr[0]["title"], "Start here");

    handle.abort();
}

#[tokio::test]
async fn test_search_returns_hits() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    // "alpha" is in node id, name, summary and tags — fuzzy matcher
    // should at least surface the alpha node.
    let resp = reqwest::get(format!("{base}/api/search?q=alpha"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let hits = body.as_array().expect("search must return an array");
    assert!(!hits.is_empty(), "expected at least one hit for 'alpha'");
    // Every hit must carry an id (string) and score (number).
    for h in hits {
        assert!(h["id"].is_string(), "hit missing id: {h}");
        assert!(h["score"].is_number(), "hit missing score: {h}");
    }
    // alpha must show up — id-substring is the strongest signal.
    let ids: Vec<&str> = hits.iter().filter_map(|h| h["id"].as_str()).collect();
    assert!(
        ids.contains(&"alpha"),
        "expected 'alpha' in hits, got {ids:?}"
    );

    handle.abort();
}

#[tokio::test]
async fn test_search_with_type_filter() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    // Filter to functions — "alpha" is a function, "beta" is a class.
    // Even with a query that matches both summaries, only alpha should
    // come back.
    let resp = reqwest::get(format!("{base}/api/search?q=the&type=function"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let hits: serde_json::Value = resp.json().await.unwrap();
    let arr = hits.as_array().unwrap();
    let ids: Vec<&str> = arr.iter().filter_map(|h| h["id"].as_str()).collect();
    assert!(
        !ids.contains(&"beta"),
        "type=function should filter out the class node beta, got {ids:?}"
    );

    handle.abort();
}

#[tokio::test]
async fn test_node_returns_clone() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/node?id=alpha"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], "alpha");
    assert_eq!(body["name"], "alpha_fn");
    assert_eq!(body["type"], "function");
    assert_eq!(body["filePath"], "src/alpha.rs");

    handle.abort();
}

#[tokio::test]
async fn test_node_404_on_missing() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/node?id=does_not_exist"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    handle.abort();
}

#[tokio::test]
async fn test_neighbors_returns_subgraph() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    // The route currently ignores `depth` (handler-side TODO) but we
    // still pass it to document the wire shape clients use.
    let resp = reqwest::get(format!("{base}/api/neighbors?id=alpha&depth=1"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["center"]["id"], "alpha");
    let neighbors = body["neighbors"].as_array().unwrap();
    let neighbor_ids: Vec<&str> = neighbors
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert_eq!(neighbor_ids, vec!["beta"]);
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["source"], "alpha");
    assert_eq!(edges[0]["target"], "beta");

    handle.abort();
}

#[tokio::test]
async fn test_neighbors_404_on_missing_node() {
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/neighbors?id=does_not_exist"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);

    handle.abort();
}

#[tokio::test]
async fn test_layers_honours_kind_query() {
    // No domain overlay loaded — `kind=domain` must 404 with a body
    // mentioning the missing kind. Mirrors the contract that
    // `tests/api.rs::graph_endpoint_honours_kind_param` documents for
    // `/api/graph`.
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/layers?kind=domain"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("domain"),
        "expected 404 body to mention the missing kind, got: {text}"
    );

    handle.abort();
}

#[tokio::test]
async fn test_tour_honours_kind_query() {
    // Same shape as the layers variant.
    let state = AppState::with_graphs(populated_graph("p"), None, None, PathBuf::new());
    let (base, handle) = boot_with(state).await;

    let resp = reqwest::get(format!("{base}/api/tour?kind=domain"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("domain"),
        "expected 404 body to mention the missing kind, got: {text}"
    );

    handle.abort();
}
