//! Workspace-wide error type.
//!
//! Single `ua_core::Error` carrying every variant produced by the
//! storage / settings / extract / llm layers. Crates re-export this as
//! their canonical error and mark their old per-crate enums as
//! deprecated aliases.
//!
//! Keep this enum `#[non_exhaustive]` so adding new variants is
//! source-compatible across the workspace.

use crate::validate::ValidationError;

/// One workspace-wide error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    // ---- generic IO + serialization -------------------------------------
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error("bincode: {0}")]
    Bincode(String),
    #[error("tar: {0}")]
    Tar(String),
    #[error("zstd: {0}")]
    Zstd(String),

    // ---- validation -----------------------------------------------------
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),

    // ---- settings -------------------------------------------------------
    #[error("settings: {0}")]
    Settings(String),

    // ---- storage --------------------------------------------------------
    /// Free-form storage error (graph dump, archive op, etc).
    #[error("storage: {0}")]
    Storage(String),
    /// In-memory graph backend (IndraDB-style failures).
    #[error("graph: {0}")]
    Graph(String),
    /// HNSW failure (index build or search).
    #[error("hnsw: {0}")]
    Hnsw(String),
    /// Schema/internal version mismatch when re-opening an archive.
    #[error("schema: {0}")]
    Schema(String),
    /// New embedding dimension does not match the stored one.
    #[error(
        "embedding dimension mismatch: stored={stored} new={new} for model `{model}`. \
         Run `understandable embed --reset` to switch models."
    )]
    EmbeddingDimMismatch {
        model: String,
        stored: usize,
        new: usize,
    },
    /// Stored project root differs from the current layout's root —
    /// almost always two projects sharing the same `storage.dir`.
    #[error(
        "project root mismatch: stored=`{stored}` current=`{current}`. \
         Two projects may be sharing the same storage directory. Pick distinct \
         `storage.dir` / `storage.db_name` values in `understandable.yaml`."
    )]
    ProjectRootMismatch { stored: String, current: String },

    // ---- LLM / providers ------------------------------------------------
    #[error("anthropic: {0}")]
    Anthropic(String),
    #[error("provider: {0}")]
    Provider(String),
    #[error("plugin: {0}")]
    Plugin(String),
    #[error("local embedding: {0}")]
    LocalEmbedding(String),
    #[error("embedding: {0}")]
    Embedding(String),

    /// Anything that doesn't fit a more specific bucket.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Build a free-form storage error.
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Storage(msg.into())
    }

    /// Build a free-form schema error.
    pub fn schema(msg: impl Into<String>) -> Self {
        Self::Schema(msg.into())
    }

    /// Build a free-form graph error.
    pub fn graph(msg: impl Into<String>) -> Self {
        Self::Graph(msg.into())
    }

    /// Build a free-form HNSW error.
    pub fn hnsw(msg: impl Into<String>) -> Self {
        Self::Hnsw(msg.into())
    }
}
