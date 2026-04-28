//! JSON API mounted under `/api`.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use futures::stream::Stream;
use futures::StreamExt as _;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;
use ua_core::{EdgeType, GraphEdge, GraphNode, KnowledgeGraph, NodeType};

use crate::state::{AppState, GraphKind};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/graph", get(full_graph))
        .route("/api/graph/nodes", get(list_nodes))
        .route("/api/graph/edges", get(list_edges))
        .route("/api/project", get(project_meta))
        .route("/api/layers", get(layers))
        .route("/api/tour", get(tour))
        .route("/api/search", get(search))
        .route("/api/source", get(get_source))
        .route("/api/node", get(node_by_id))
        .route("/api/neighbors", get(node_neighbors))
        .route("/api/diff", get(diff_overlay))
        .route("/api/events", get(events))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))
}

#[derive(Deserialize, Default)]
pub struct KindQuery {
    pub kind: Option<String>,
}

#[allow(clippy::result_large_err)]
fn parse_kind(raw: Option<&str>) -> Result<GraphKind, Response> {
    let s = raw.unwrap_or("codebase").trim();
    match GraphKind::from_query(s) {
        Some(k) => Ok(k),
        None => Err((
            StatusCode::BAD_REQUEST,
            format!("unknown kind '{s}'; expected one of: codebase, domain, knowledge"),
        )
            .into_response()),
    }
}

/// Resolve the requested kind to a cached graph. Returns 404 with a
/// descriptive message when the kind has no graph loaded.
#[allow(clippy::result_large_err)]
fn graph_for_kind(state: &AppState, raw: Option<&str>) -> Result<Arc<KnowledgeGraph>, Response> {
    let kind = parse_kind(raw)?;
    state.graph_for(kind).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!(
                "no '{}' graph loaded for this project; run `understandable {}` first",
                kind.as_str(),
                match kind {
                    GraphKind::Codebase => "analyze",
                    GraphKind::Domain => "domain",
                    GraphKind::Knowledge => "knowledge",
                }
            ),
        )
            .into_response()
    })
}

async fn full_graph(State(state): State<Arc<AppState>>, Query(q): Query<KindQuery>) -> Response {
    match graph_for_kind(&state, q.kind.as_deref()) {
        Ok(g) => Json(g.as_ref()).into_response(),
        Err(r) => r,
    }
}

async fn project_meta(State(state): State<Arc<AppState>>) -> Json<ua_core::ProjectMeta> {
    Json(state.primary_graph().project.clone())
}

async fn layers(State(state): State<Arc<AppState>>, Query(q): Query<KindQuery>) -> Response {
    match graph_for_kind(&state, q.kind.as_deref()) {
        Ok(g) => Json(&g.as_ref().layers).into_response(),
        Err(r) => r,
    }
}

async fn tour(State(state): State<Arc<AppState>>, Query(q): Query<KindQuery>) -> Response {
    match graph_for_kind(&state, q.kind.as_deref()) {
        Ok(g) => Json(&g.as_ref().tour).into_response(),
        Err(r) => r,
    }
}

/// Return the diff-overlay JSON if it exists in the storage directory,
/// otherwise 204 No Content. We don't parse the file — pass it through
/// verbatim so the dashboard schema can evolve without churning the
/// server.
async fn diff_overlay(State(state): State<Arc<AppState>>) -> Response {
    if state.storage_dir.as_os_str().is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }
    let path = state.storage_dir.join("diff-overlay.json");
    match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(v) => Json(v).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("diff-overlay.json is not valid JSON: {e}"),
            )
                .into_response(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read diff-overlay.json: {e}"),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct NodeQuery {
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    pub layer: Option<String>,
    pub q: Option<String>,
    pub kind: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    250
}

#[derive(serde::Serialize)]
pub struct PageMeta {
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(serde::Serialize)]
pub struct NodePage<'a> {
    pub items: Vec<&'a GraphNode>,
    #[serde(flatten)]
    pub meta: PageMeta,
}

async fn list_nodes(State(state): State<Arc<AppState>>, Query(q): Query<NodeQuery>) -> Response {
    let graph = match graph_for_kind(&state, q.kind.as_deref()) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let allowed = parse_node_type(q.type_filter.as_deref());
    let layer_ids: Option<Vec<&str>> = q.layer.as_deref().map(|name| {
        graph
            .layers
            .iter()
            .filter(|l| l.id == name || l.name == name)
            .flat_map(|l| l.node_ids.iter().map(|s| s.as_str()))
            .collect()
    });
    let needle = q.q.as_deref().map(str::to_lowercase);

    let mut items: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| match (&allowed, n.node_type) {
            (Some(t), nt) => *t == nt,
            (None, _) => true,
        })
        .filter(|n| match &layer_ids {
            Some(ids) => ids.iter().any(|id| *id == n.id),
            None => true,
        })
        .filter(|n| match &needle {
            Some(needle) => {
                n.name.to_lowercase().contains(needle)
                    || n.summary.to_lowercase().contains(needle)
                    || n.tags.iter().any(|t| t.to_lowercase().contains(needle))
            }
            None => true,
        })
        .collect();
    let total = items.len();
    // `offset + limit` can overflow `usize` for adversarial inputs;
    // saturate so we always end up with a sane window.
    let end = q.offset.saturating_add(q.limit).min(items.len());
    let start = q.offset.min(end);
    let page: Vec<&GraphNode> = items.drain(start..end).collect();

    Json(NodePage {
        items: page,
        meta: PageMeta {
            total,
            limit: q.limit,
            offset: q.offset,
        },
    })
    .into_response()
}

#[derive(Deserialize)]
pub struct EdgeQuery {
    pub source: Option<String>,
    pub target: Option<String>,
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    pub kind: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(serde::Serialize)]
pub struct EdgePage<'a> {
    pub items: Vec<&'a GraphEdge>,
    #[serde(flatten)]
    pub meta: PageMeta,
}

async fn list_edges(State(state): State<Arc<AppState>>, Query(q): Query<EdgeQuery>) -> Response {
    let graph = match graph_for_kind(&state, q.kind.as_deref()) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let allowed = parse_edge_type(q.type_filter.as_deref());
    let mut items: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| match &q.source {
            Some(s) => e.source == *s,
            None => true,
        })
        .filter(|e| match &q.target {
            Some(t) => e.target == *t,
            None => true,
        })
        .filter(|e| match (&allowed, e.edge_type) {
            (Some(t), et) => *t == et,
            (None, _) => true,
        })
        .collect();
    let total = items.len();
    let end = q.offset.saturating_add(q.limit).min(items.len());
    let start = q.offset.min(end);
    let page: Vec<&GraphEdge> = items.drain(start..end).collect();

    Json(EdgePage {
        items: page,
        meta: PageMeta {
            total,
            limit: q.limit,
            offset: q.offset,
        },
    })
    .into_response()
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(rename = "type")]
    pub type_filter: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    25
}

#[derive(serde::Serialize)]
pub struct SearchHit {
    pub id: String,
    pub score: f32,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Json<Vec<SearchHit>> {
    let types = match parse_node_type(q.type_filter.as_deref()) {
        Some(t) => vec![t],
        None => Vec::new(),
    };
    let opts = ua_search::SearchOptions {
        types,
        limit: Some(q.limit),
    };
    let hits = state.with_search(|engine| {
        engine
            .search(&q.q, &opts)
            .into_iter()
            .map(|r| SearchHit {
                id: r.node_id,
                score: r.score,
            })
            .collect::<Vec<_>>()
    });
    Json(hits)
}

#[derive(Deserialize)]
pub struct NodeIdQuery {
    pub id: String,
}

async fn node_by_id(State(state): State<Arc<AppState>>, Query(q): Query<NodeIdQuery>) -> Response {
    let graph = state.primary_graph();
    match graph.nodes.iter().find(|n| n.id == q.id) {
        Some(n) => Json(n.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "node not found").into_response(),
    }
}

#[derive(Deserialize)]
pub struct NeighborQuery {
    pub id: String,
    /// Hop count for neighbour expansion. Defaults to 1; capped at 4 to
    /// keep response sizes bounded on heavily connected nodes.
    pub depth: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct NeighborhoodResponse<'a> {
    pub center: &'a GraphNode,
    pub neighbors: Vec<&'a GraphNode>,
    pub edges: Vec<&'a GraphEdge>,
}

async fn node_neighbors(
    State(state): State<Arc<AppState>>,
    Query(q): Query<NeighborQuery>,
) -> Response {
    let graph = state.primary_graph();
    let id = q.id.as_str();
    let Some(center) = graph.nodes.iter().find(|n| n.id == id) else {
        return (StatusCode::NOT_FOUND, "node not found").into_response();
    };
    let depth = q.depth.unwrap_or(1).clamp(1, 4);

    // BFS over the directed-but-symmetric adjacency. We track visited
    // node ids and the edges traversed so the response includes every
    // edge the dashboard needs to render the sub-graph.
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut frontier: Vec<&str> = vec![id];
    visited.insert(id);
    let mut edge_set: Vec<&GraphEdge> = Vec::new();
    let mut seen_edge: std::collections::HashSet<(*const GraphEdge,)> =
        std::collections::HashSet::new();

    for _ in 0..depth {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<&str> = Vec::new();
        for &cur in frontier.iter() {
            for e in graph.edges.iter() {
                if e.source != cur && e.target != cur {
                    continue;
                }
                let key = (e as *const GraphEdge,);
                if seen_edge.insert(key) {
                    edge_set.push(e);
                }
                let other = if e.source == cur {
                    e.target.as_str()
                } else {
                    e.source.as_str()
                };
                if visited.insert(other) {
                    next.push(other);
                }
            }
        }
        frontier = next;
    }
    visited.remove(id);
    let neighbors: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| visited.contains(n.id.as_str()))
        .collect();
    Json(NeighborhoodResponse {
        center,
        neighbors,
        edges: edge_set,
    })
    .into_response()
}

/// Maximum number of bytes returned by `/api/source`. Anything larger
/// is rejected with `413 Payload Too Large`. Keeps the dashboard from
/// chewing through memory on a runaway repository.
const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
pub struct SourceQuery {
    pub path: String,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

/// Resolve `requested` against `root`, rejecting traversal attempts
/// (`..`), absolute paths, and any final path that escapes the root
/// after canonicalisation. Returns the canonical path on success.
#[allow(clippy::result_large_err)]
fn sanitize_source_path(requested: &str, root: &Path) -> Result<std::path::PathBuf, Response> {
    if root.as_os_str().is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "server has no project root configured; /api/source disabled",
        )
            .into_response());
    }
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing 'path' query parameter").into_response());
    }
    let p = Path::new(trimmed);
    // Reject any explicit traversal segment up front; even on Windows
    // we never want `..` to slip through.
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err((StatusCode::BAD_REQUEST, "path contains '..'; rejected").into_response());
    }
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    // canonicalise — also confirms the file exists.
    let canon = match std::fs::canonicalize(&joined) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(
                (StatusCode::NOT_FOUND, format!("file not found: {trimmed}")).into_response(),
            );
        }
        Err(e) => {
            return Err(
                (StatusCode::BAD_REQUEST, format!("cannot resolve path: {e}")).into_response(),
            );
        }
    };
    if !canon.starts_with(root) {
        return Err((
            StatusCode::BAD_REQUEST,
            "path escapes project root; rejected",
        )
            .into_response());
    }
    Ok(canon)
}

async fn get_source(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SourceQuery>,
) -> Response {
    let path = match sanitize_source_path(&params.path, &state.project_root) {
        Ok(p) => p,
        Err(r) => return r,
    };

    // Cheap pre-check: avoid loading multi-MB files into memory just to
    // reject them. We still re-check after read because metadata can
    // race with the read on a busy filesystem.
    if let Ok(meta) = tokio::fs::metadata(&path).await {
        if meta.len() as usize > MAX_SOURCE_BYTES {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "source file exceeds {MAX_SOURCE_BYTES}-byte cap (size={})",
                    meta.len()
                ),
            )
                .into_response();
        }
    }

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to read file: {e}"),
            )
                .into_response();
        }
    };
    if content.len() > MAX_SOURCE_BYTES {
        return (StatusCode::PAYLOAD_TOO_LARGE, "source file exceeds 1MB cap").into_response();
    }

    let slice = match (params.start, params.end) {
        (Some(start), Some(end)) if end >= start && start >= 1 => content
            .lines()
            .skip(start.saturating_sub(1))
            .take(end - start + 1)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => content,
    };

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        slice,
    )
        .into_response()
}

/// Server-Sent Events endpoint. Clients subscribe here and receive a
/// `graph-reloaded` event whenever the file watcher detects an updated
/// archive and successfully reloads the in-memory graph.
async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        let ev = res.ok()?;
        // Serialize `ReloadEvent` as the JSON data field.
        let data = serde_json::to_string(&ev).ok()?;
        Some(Ok(SseEvent::default().event("graph-reloaded").data(data)))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

fn parse_node_type(s: Option<&str>) -> Option<NodeType> {
    let s = s?.trim();
    NodeType::ALL.iter().copied().find(|t| t.as_str() == s)
}

fn parse_edge_type(s: Option<&str>) -> Option<EdgeType> {
    let s = s?.trim();
    EdgeType::ALL.iter().copied().find(|t| edge_label(*t) == s)
}

fn edge_label(t: EdgeType) -> &'static str {
    use EdgeType::*;
    match t {
        Imports => "imports",
        Exports => "exports",
        Contains => "contains",
        Inherits => "inherits",
        Implements => "implements",
        Calls => "calls",
        Subscribes => "subscribes",
        Publishes => "publishes",
        Middleware => "middleware",
        ReadsFrom => "reads_from",
        WritesTo => "writes_to",
        Transforms => "transforms",
        Validates => "validates",
        DependsOn => "depends_on",
        TestedBy => "tested_by",
        Configures => "configures",
        Related => "related",
        SimilarTo => "similar_to",
        Deploys => "deploys",
        Serves => "serves",
        Provisions => "provisions",
        Triggers => "triggers",
        Migrates => "migrates",
        Documents => "documents",
        Routes => "routes",
        DefinesSchema => "defines_schema",
        ContainsFlow => "contains_flow",
        FlowStep => "flow_step",
        CrossDomain => "cross_domain",
        Cites => "cites",
        Contradicts => "contradicts",
        BuildsOn => "builds_on",
        Exemplifies => "exemplifies",
        CategorizedUnder => "categorized_under",
        AuthoredBy => "authored_by",
    }
}
