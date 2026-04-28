//! axum-backed dashboard server.
//!
//! All three graph kinds (`codebase`, `domain`, `knowledge`) are opened
//! once per `serve` call and cached as `Arc<KnowledgeGraph>` in
//! [`AppState`]. Every endpoint is read-only — mutations happen via the
//! CLI.

pub mod assets;
pub mod routes;
pub mod state;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use axum::http::{HeaderValue, Method};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub use state::{AppState, GraphKind};

/// Build a fully wired router given a pre-loaded [`AppState`].
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(routes::api::router())
        .merge(routes::assets_router())
        .layer(cors_for(None))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Build a router using a CORS layer scoped to `bind_addr`. Loopback
/// hosts get an allowlist of `http://127.0.0.1:<port>` and
/// `http://localhost:<port>`; non-loopback hosts get the same
/// allowlist with no extra wildcard.
pub fn router_for(state: Arc<AppState>, bind_addr: SocketAddr) -> Router {
    Router::new()
        .merge(routes::api::router())
        .merge(routes::assets_router())
        .layer(cors_for(Some(bind_addr)))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Build a tightly-scoped CORS layer. We never use `permissive()` —
/// the dashboard only needs to talk to itself, and an open CORS policy
/// on a localhost server is an XSRF foot-gun.
fn cors_for(bind_addr: Option<SocketAddr>) -> CorsLayer {
    let mut origins: Vec<HeaderValue> = Vec::new();
    if let Some(addr) = bind_addr {
        let port = addr.port();
        for host in ["127.0.0.1", "localhost"] {
            if let Ok(v) = format!("http://{host}:{port}").parse::<HeaderValue>() {
                origins.push(v);
            }
        }
        // Only loopback gets the relaxed treatment of allowing both
        // hostnames; for any other bind we still only echo back the
        // exact bound origin.
        if !is_loopback(addr.ip()) {
            if let Ok(v) = format!("http://{}", addr).parse::<HeaderValue>() {
                origins.push(v);
            }
        }
    }
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Open the storage at `project_root`, snapshot every graph kind into
/// [`AppState`], then bind to `addr`. Blocks until the server exits or
/// receives Ctrl-C.
///
/// Equivalent to `serve_kind(project_root, addr, "codebase")` — kept
/// as a thin wrapper so callers that don't care about overlay kinds
/// can ignore the new parameter.
pub async fn serve(project_root: &Path, addr: SocketAddr) -> anyhow::Result<()> {
    serve_kind(project_root, addr, "codebase").await
}

/// Same as [`serve`] but loads `kind` (`"codebase"` / `"domain"` /
/// `"knowledge"`) as the primary graph backing `state.graph` and the
/// search index. The other two kinds are still loaded into their
/// optional overlay slots when their archives exist on disk.
pub async fn serve_kind(
    project_root: &Path,
    addr: SocketAddr,
    kind: &str,
) -> anyhow::Result<()> {
    let state = AppState::load_from_project_kind(project_root, kind)
        .await
        .with_context(|| format!("loading project graphs (kind={kind})"))?;
    let state = Arc::new(state);
    let app = router_for(state, addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(?addr, kind, "ua-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Resolve once Ctrl-C is observed. Errors from the signal handler are
/// logged but otherwise treated as "shut down anyway".
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::warn!(?e, "ctrl_c handler failed; exiting");
    } else {
        tracing::info!("ctrl_c received; shutting down");
    }
}
