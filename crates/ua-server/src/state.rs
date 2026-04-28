//! Server-side application state.
//!
//! Holds in-memory [`KnowledgeGraph`]s (one per kind: codebase / domain /
//! knowledge) plus a reusable [`SearchEngine`] over the codebase graph
//! for the lifetime of the process. Each graph slot is wrapped in
//! [`arc_swap::ArcSwap`] so HTTP handlers can load a snapshot without
//! holding a lock, and the file-watcher task can swap a freshly-loaded
//! graph in atomically at any time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::Serialize;
use tokio::sync::broadcast;
use ua_core::KnowledgeGraph;
use ua_persist::{ProjectLayout, Storage};
use ua_search::SearchEngine;

/// Capacity of the broadcast channel. Each new subscriber gets a copy of
/// the last `BROADCAST_CAP` events so they don't miss a reload that fired
/// right before they connected. In practice we send at most a handful of
/// events per minute, so 16 is ample.
const BROADCAST_CAP: usize = 16;

/// Logical kinds that the dashboard can serve. Mirrors
/// `ProjectLayout::graph_archive_for`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphKind {
    Codebase,
    Domain,
    Knowledge,
}

impl GraphKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphKind::Codebase => "codebase",
            GraphKind::Domain => "domain",
            GraphKind::Knowledge => "knowledge",
        }
    }

    pub fn from_query(s: &str) -> Option<Self> {
        match s {
            "codebase" | "" => Some(GraphKind::Codebase),
            "domain" => Some(GraphKind::Domain),
            "knowledge" => Some(GraphKind::Knowledge),
            _ => None,
        }
    }
}

/// The payload pushed to all SSE subscribers when a graph is reloaded.
#[derive(Clone, Debug, Serialize)]
pub struct ReloadEvent {
    pub kind: String,
}

pub struct AppState {
    /// Per-kind graph slots. Keyed by `"codebase"`, `"domain"`,
    /// `"knowledge"`. Each slot is an `ArcSwap` so the watcher can do
    /// a zero-copy atomic swap while readers hold their own `Arc`s.
    graphs: HashMap<String, ArcSwap<Option<KnowledgeGraph>>>,
    /// Search index over the primary (codebase) graph. Rebuilt on each
    /// codebase reload under a mutex.
    search: std::sync::Mutex<SearchEngine>,
    /// Storage directory; used to look up overlay artefacts like
    /// `diff-overlay.json`.
    pub storage_dir: PathBuf,
    /// Absolute, canonicalised project root. Used by `/api/source` to
    /// reject path-traversal attempts: every requested file must
    /// canonicalise inside this prefix. May be empty in tests, in which
    /// case `/api/source` rejects all reads.
    pub project_root: PathBuf,
    /// Layout used by [`Self::reload_kind`] to locate archives.
    layout: ProjectLayout,
    /// Broadcast sender for live-reload SSE events. Cloning the sender
    /// does *not* create a new channel — all clones share the same one.
    pub tx: broadcast::Sender<ReloadEvent>,
}

// `ArcSwap<T>` is Send+Sync when T: Send+Sync. `KnowledgeGraph` is
// Send+Sync, and `Option<KnowledgeGraph>` is too.
// `broadcast::Sender<T>` is also Send+Sync when T: Send+Sync.
// `std::sync::Mutex<SearchEngine>` is Send+Sync when SearchEngine: Send.
// All of the above hold, so AppState is Send+Sync.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AppState>();
};

impl AppState {
    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn make_graphs(
        codebase: KnowledgeGraph,
        domain: Option<KnowledgeGraph>,
        knowledge: Option<KnowledgeGraph>,
    ) -> HashMap<String, ArcSwap<Option<KnowledgeGraph>>> {
        let mut m = HashMap::new();
        m.insert(
            "codebase".to_string(),
            ArcSwap::new(Arc::new(Some(codebase))),
        );
        m.insert("domain".to_string(), ArcSwap::new(Arc::new(domain)));
        m.insert("knowledge".to_string(), ArcSwap::new(Arc::new(knowledge)));
        m
    }

    fn build(
        graphs: HashMap<String, ArcSwap<Option<KnowledgeGraph>>>,
        storage_dir: PathBuf,
        project_root: PathBuf,
        layout: ProjectLayout,
    ) -> Self {
        // Seed the search engine from whatever is in the codebase slot.
        let search_nodes = graphs
            .get("codebase")
            .and_then(|s| s.load().as_ref().as_ref().map(|g| g.nodes.clone()))
            .unwrap_or_default();
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        Self {
            graphs,
            search: std::sync::Mutex::new(SearchEngine::new(search_nodes)),
            storage_dir,
            project_root,
            layout,
            tx,
        }
    }

    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Construct from a pre-loaded codebase graph. Domain/knowledge slots
    /// stay empty. Mostly useful in tests.
    pub fn new(graph: KnowledgeGraph) -> Self {
        let layout = ProjectLayout::under(PathBuf::new());
        let graphs = Self::make_graphs(graph, None, None);
        Self::build(graphs, PathBuf::new(), PathBuf::new(), layout)
    }

    /// Build with the full set of pre-loaded graphs and a storage
    /// directory pointer for overlay file lookups.
    pub fn with_graphs(
        graph: KnowledgeGraph,
        domain: Option<KnowledgeGraph>,
        knowledge: Option<KnowledgeGraph>,
        storage_dir: PathBuf,
    ) -> Self {
        let layout = ProjectLayout::under(PathBuf::new());
        let graphs = Self::make_graphs(graph, domain, knowledge);
        Self::build(graphs, storage_dir, PathBuf::new(), layout)
    }

    /// Variant of [`Self::with_graphs`] that also records a project root
    /// for `/api/source`. Tests that exercise the source endpoint use
    /// this; production code paths flow through
    /// [`Self::load_from_project_kind`] which already populates the
    /// field.
    pub fn with_graphs_and_root(
        graph: KnowledgeGraph,
        domain: Option<KnowledgeGraph>,
        knowledge: Option<KnowledgeGraph>,
        storage_dir: PathBuf,
        project_root: PathBuf,
    ) -> Self {
        let layout = ProjectLayout::under(PathBuf::new());
        let graphs = Self::make_graphs(graph, domain, knowledge);
        Self::build(graphs, storage_dir, project_root, layout)
    }

    /// Open every per-kind store under `project_root`. Missing kinds
    /// surface as `None`. The codebase store is always opened (the
    /// schema bootstrap creates an empty DB if no `*.db.zst` exists).
    pub async fn load_from_project(project_root: &Path) -> anyhow::Result<Self> {
        Self::load_from_project_kind(project_root, "codebase").await
    }

    /// Same as [`Self::load_from_project`] but treats `primary_kind`
    /// (one of `"codebase"`, `"domain"`, `"knowledge"`) as the primary
    /// graph that backs `state.graph` and the search index. The other
    /// two kinds still load into their optional slots when their
    /// archives exist on disk so the cross-kind `?kind=` query keeps
    /// working.
    pub async fn load_from_project_kind(
        project_root: &Path,
        primary_kind: &str,
    ) -> anyhow::Result<Self> {
        let layout = ProjectLayout::for_project(project_root);

        // Primary: required. For "codebase" the underlying
        // `Storage::open_kind` will create an empty schema if no
        // compressed archive is on disk yet; for the overlay kinds
        // we expect the archive to already exist (the CLI verifies
        // before calling `serve_kind`, but we re-check here so the
        // library is hard to misuse).
        if primary_kind != "codebase" && !layout.graph_archive_for(primary_kind).exists() {
            anyhow::bail!(
                "no archive at {} — run `understandable {}` first",
                layout.graph_archive_for(primary_kind).display(),
                primary_kind,
            );
        }
        let primary_storage = Storage::open_kind(&layout, primary_kind).await?;
        let primary = primary_storage.load_graph().await?;

        // Mirror the primary into its matching overlay slot (so
        // `graph_for(GraphKind::<primary>)` keeps working) and try to
        // load the other two kinds into their slots. We don't reuse
        // the primary archive twice — the redundant load only happens
        // for the *other* kinds, and any of them missing on disk just
        // surfaces as `None`.
        let (domain, knowledge) = match primary_kind {
            "domain" => (
                Some(primary.clone()),
                load_optional_kind(&layout, "knowledge").await,
            ),
            "knowledge" => (
                load_optional_kind(&layout, "domain").await,
                Some(primary.clone()),
            ),
            _ => (
                load_optional_kind(&layout, "domain").await,
                load_optional_kind(&layout, "knowledge").await,
            ),
        };

        // Canonicalise the project root once, here, so every
        // `/api/source` lookup can compare prefixes cheaply without
        // touching the filesystem again. Falls back to the raw path if
        // canonicalisation fails (e.g. on a deleted dir during tests).
        let canonical_root =
            std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());

        let storage_dir = layout.root.clone();
        let graphs = Self::make_graphs(primary, domain, knowledge);
        Ok(Self::build(graphs, storage_dir, canonical_root, layout))
    }

    // -----------------------------------------------------------------------
    // Runtime graph access
    // -----------------------------------------------------------------------

    /// Pick a cached graph by kind. Returns `None` when the slot is empty
    /// (i.e. the corresponding `understandable <kind>` command has never
    /// been run). The caller receives an owned `Arc` and can hold it across
    /// await points without blocking the watcher.
    pub fn graph_for(&self, kind: GraphKind) -> Option<Arc<KnowledgeGraph>> {
        let slot = self.graphs.get(kind.as_str())?;
        // `load_full()` increments the reference count and gives us an
        // `Arc<Option<KnowledgeGraph>>`. We then flatten it into
        // `Option<Arc<KnowledgeGraph>>`.
        let arc_opt: Arc<Option<KnowledgeGraph>> = slot.load_full();
        // We can't move out of an Arc, so we clone the inner graph.
        arc_opt.as_ref().as_ref().map(|g| Arc::new(g.clone()))
    }

    /// Borrow the search engine for the duration of a closure. Returns
    /// the closure's return value, or `Default::default()` if the mutex
    /// is poisoned.
    pub fn with_search<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&SearchEngine) -> R,
        R: Default,
    {
        match self.search.lock() {
            Ok(guard) => f(&guard),
            Err(_) => R::default(),
        }
    }

    // -----------------------------------------------------------------------
    // Live-reload
    // -----------------------------------------------------------------------

    /// Reload a single graph kind from its archive on disk and broadcast
    /// a [`ReloadEvent`] to all SSE subscribers. Called by the file-watcher
    /// task; safe to call from any async context.
    pub async fn reload_kind(&self, kind: &str) -> anyhow::Result<()> {
        let archive = self.layout.graph_archive_for(kind);
        if !archive.exists() {
            // Archive removed between the watcher event and now — leave
            // the in-memory graph in place (stale but non-crashing).
            return Ok(());
        }
        let storage = Storage::open_kind(&self.layout, kind).await?;
        let graph = storage.load_graph().await?;

        // If this is the codebase graph, rebuild the search index too.
        if kind == "codebase" {
            let new_engine = SearchEngine::new(graph.nodes.clone());
            if let Ok(mut guard) = self.search.lock() {
                *guard = new_engine;
            }
        }

        // Atomically swap the slot. All `Arc` clones that handlers
        // already hold keep pointing at the old graph until they drop.
        if let Some(slot) = self.graphs.get(kind) {
            slot.store(Arc::new(Some(graph)));
        }

        // Notify SSE subscribers. `send` fails only if there are zero
        // active receivers — that's fine, just ignore it.
        let _ = self.tx.send(ReloadEvent {
            kind: kind.to_string(),
        });

        Ok(())
    }

    /// The primary graph (always `"codebase"` slot) — used by routes that
    /// don't accept a `?kind=` parameter (e.g. `/api/project`).
    ///
    /// Returns `None` only in the extremely unlikely scenario that the
    /// codebase slot was never populated (shouldn't happen in production;
    /// callers should handle `None` gracefully).
    pub fn primary_graph(&self) -> Arc<KnowledgeGraph> {
        // The codebase slot is always populated by every constructor, so
        // the unwrap_or_else path is a safety net for tests that somehow
        // build a state without a codebase graph.
        self.graph_for(GraphKind::Codebase).unwrap_or_else(|| {
            Arc::new(KnowledgeGraph::new(ua_core::ProjectMeta {
                name: String::new(),
                languages: vec![],
                frameworks: vec![],
                description: String::new(),
                analyzed_at: String::new(),
                git_commit_hash: String::new(),
            }))
        })
    }
}

async fn load_optional_kind(layout: &ProjectLayout, kind: &str) -> Option<KnowledgeGraph> {
    // If there's no compressed file on disk we don't need to spin up an
    // empty DB just to return an empty graph — short-circuit and report
    // "not present".
    if !layout.graph_archive_for(kind).exists() {
        return None;
    }
    match Storage::open_kind(layout, kind).await {
        Ok(s) => match s.load_graph().await {
            Ok(g) => Some(g),
            Err(e) => {
                tracing::warn!(?e, kind, "failed to load overlay graph");
                None
            }
        },
        Err(e) => {
            tracing::warn!(?e, kind, "failed to open overlay storage");
            None
        }
    }
}
