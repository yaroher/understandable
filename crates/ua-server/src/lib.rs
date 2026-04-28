//! axum-backed dashboard server.
//!
//! All three graph kinds (`codebase`, `domain`, `knowledge`) are opened
//! once per `serve` call and cached as `Arc<KnowledgeGraph>` in
//! [`AppState`]. Every endpoint is read-only — mutations happen via the
//! CLI.
//!
//! A background file-watcher task monitors the storage directory for
//! archive updates and reloads the in-memory graph automatically, sending
//! an SSE event to all connected dashboard clients.

pub mod assets;
pub mod routes;
pub mod state;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
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
    CorsLayer::new().allow_origin(origins).allow_methods([
        Method::GET,
        Method::HEAD,
        Method::OPTIONS,
    ])
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
pub async fn serve_kind(project_root: &Path, addr: SocketAddr, kind: &str) -> anyhow::Result<()> {
    let state = AppState::load_from_project_kind(project_root, kind)
        .await
        .with_context(|| format!("loading project graphs (kind={kind})"))?;
    let state = Arc::new(state);

    // Spawn the live-reload file watcher. If the storage directory doesn't
    // exist yet (first run before `understandable analyze`) it logs a
    // warning and skips watching — the server still works for static data.
    spawn_watcher(Arc::clone(&state));

    let app = router_for(Arc::clone(&state), addr);
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

// ---------------------------------------------------------------------------
// File watcher (live-reload)
// ---------------------------------------------------------------------------

/// Spawn a detached tokio task that watches the storage directory for
/// archive modifications and triggers [`AppState::reload_kind`].
///
/// The watcher is non-recursive: we only care about `.tar.zst` files
/// placed directly in the storage dir, not deep sub-tree changes.
///
/// Errors setting up the watcher are non-fatal: the server continues
/// to serve the initial in-memory graphs; only live-reload is disabled.
fn spawn_watcher(state: Arc<AppState>) {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    let storage_dir = state.storage_dir.clone();

    if !storage_dir.exists() {
        tracing::warn!(
            dir = %storage_dir.display(),
            "storage directory does not exist; live-reload disabled"
        );
        return;
    }

    // `notify` callbacks run on a thread-pool thread; bridge into tokio
    // via an unbounded mpsc channel.
    let (tx_evt, mut rx_evt) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();

    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(ev) = res {
            if matches!(ev.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                for p in ev.paths {
                    let _ = tx_evt.send(p);
                }
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create file watcher; live-reload disabled");
            return;
        }
    };

    if let Err(e) = watcher.watch(&storage_dir, RecursiveMode::NonRecursive) {
        tracing::warn!(
            error = %e,
            dir = %storage_dir.display(),
            "failed to watch storage directory; live-reload disabled"
        );
        return;
    }

    // Clone the layout's db_name to pass into the async task.
    // We re-derive it by looking at archive names that match the known pattern.
    tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of this task.
        let _watcher = watcher;

        // Debounce table: kind → earliest instant at which we should fire.
        let mut pending: HashMap<String, tokio::time::Instant> = HashMap::new();
        let debounce = tokio::time::Duration::from_millis(500);
        let tick = tokio::time::Duration::from_millis(100);

        loop {
            tokio::select! {
                msg = rx_evt.recv() => {
                    match msg {
                        Some(path) => {
                            if let Some(kind) = kind_from_path(&path) {
                                let deadline = tokio::time::Instant::now() + debounce;
                                pending.insert(kind, deadline);
                            }
                        }
                        None => break, // sender dropped; shut down
                    }
                }
                _ = tokio::time::sleep(tick) => {
                    let now = tokio::time::Instant::now();
                    let ready: Vec<String> = pending
                        .iter()
                        .filter(|(_, &deadline)| now >= deadline)
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in ready {
                        pending.remove(&k);
                        match state.reload_kind(&k).await {
                            Err(e) => tracing::warn!(error = %e, kind = %k, "live-reload failed"),
                            Ok(()) => tracing::info!(kind = %k, "graph reloaded (live-reload)"),
                        }
                    }
                }
            }
        }
    });
}

/// Infer the graph kind from an archive path by inspecting the filename.
///
/// Known patterns (where `<stem>` is the db_name, usually `"graph"`):
/// - `<stem>.tar.zst`              → `"codebase"`
/// - `<stem>.domain.tar.zst`       → `"domain"`
/// - `<stem>.knowledge.tar.zst`    → `"knowledge"`
fn kind_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    // Must end in .tar.zst
    let stem = name.strip_suffix(".tar.zst")?;

    // Check for overlay suffixes first (most specific first).
    for kind in &["domain", "knowledge"] {
        if stem.ends_with(&format!(".{kind}")) {
            return Some(kind.to_string());
        }
    }

    // If no overlay suffix and it otherwise looks like a graph archive
    // (non-empty stem, no extra dots that we don't recognise), treat it
    // as codebase.
    if !stem.is_empty() {
        return Some("codebase".to_string());
    }

    None
}
