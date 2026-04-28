//! Server-side application state.
//!
//! Holds in-memory [`KnowledgeGraph`]s (one per kind: codebase / domain /
//! knowledge) plus a reusable [`SearchEngine`] over the codebase graph
//! for the lifetime of the process. Each graph is wrapped in `Arc` so
//! HTTP handlers can serialise it without cloning.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ua_core::KnowledgeGraph;
use ua_persist::{ProjectLayout, Storage};
use ua_search::SearchEngine;

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

pub struct AppState {
    /// The canonical codebase graph. Always present (may be empty).
    pub graph: Arc<KnowledgeGraph>,
    /// Optional domain-overlay graph, if a domain DB exists.
    pub domain_graph: Option<Arc<KnowledgeGraph>>,
    /// Optional knowledge-overlay graph, if a knowledge DB exists.
    pub knowledge_graph: Option<Arc<KnowledgeGraph>>,
    /// Search index built on the codebase graph nodes.
    pub search: SearchEngine,
    /// Storage directory; used to look up overlay artefacts like
    /// `diff-overlay.json`.
    pub storage_dir: PathBuf,
    /// Absolute, canonicalised project root. Used by `/api/source` to
    /// reject path-traversal attempts: every requested file must
    /// canonicalise inside this prefix. May be empty in tests, in which
    /// case `/api/source` rejects all reads.
    pub project_root: PathBuf,
}

impl AppState {
    /// Construct from a pre-loaded codebase graph. Domain/knowledge slots
    /// stay empty. Mostly useful in tests.
    pub fn new(graph: KnowledgeGraph) -> Self {
        let search = SearchEngine::new(graph.nodes.clone());
        Self {
            graph: Arc::new(graph),
            domain_graph: None,
            knowledge_graph: None,
            search,
            storage_dir: PathBuf::new(),
            project_root: PathBuf::new(),
        }
    }

    /// Build with the full set of pre-loaded graphs and a storage
    /// directory pointer for overlay file lookups.
    pub fn with_graphs(
        graph: KnowledgeGraph,
        domain: Option<KnowledgeGraph>,
        knowledge: Option<KnowledgeGraph>,
        storage_dir: PathBuf,
    ) -> Self {
        let search = SearchEngine::new(graph.nodes.clone());
        Self {
            graph: Arc::new(graph),
            domain_graph: domain.map(Arc::new),
            knowledge_graph: knowledge.map(Arc::new),
            search,
            storage_dir,
            project_root: PathBuf::new(),
        }
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
        let search = SearchEngine::new(graph.nodes.clone());
        Self {
            graph: Arc::new(graph),
            domain_graph: domain.map(Arc::new),
            knowledge_graph: knowledge.map(Arc::new),
            search,
            storage_dir,
            project_root,
        }
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
        Ok(Self::with_graphs_and_root(
            primary,
            domain,
            knowledge,
            layout.root.clone(),
            canonical_root,
        ))
    }

    /// Pick a cached graph by kind. Falls back to the primary
    /// `state.graph` when the requested overlay slot is empty but the
    /// primary graph itself was loaded for that kind (i.e. when the
    /// dashboard was booted with `--kind=domain` etc.).
    pub fn graph_for(&self, kind: GraphKind) -> Option<Arc<KnowledgeGraph>> {
        match kind {
            GraphKind::Codebase => Some(self.graph.clone()),
            GraphKind::Domain => self.domain_graph.clone(),
            GraphKind::Knowledge => self.knowledge_graph.clone(),
        }
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
