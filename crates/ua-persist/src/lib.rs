//! IndraDB + usearch + tar.zst persistence for the `understandable`
//! knowledge graph.
//!
//! ## On-disk layout (one analyzed project)
//!
//! ```text
//! .understandable/
//!   graph.tar.zst       # canonical store (graph + embeddings + metadata)
//!   meta.json           # last-analyzed metadata (small, plain JSON)
//!   config.json         # {autoUpdate}
//! ```
//!
//! Working with a project goes through three steps:
//!   1. `Storage::open(path)` — decompress + untar `graph.tar.zst`
//!      into a fresh in-memory `MemoryDatastore` plus
//!      `EmbeddingsState`;
//!   2. queries / mutations via `Storage` methods (everything is
//!      in-memory);
//!   3. `storage.save(path)` — re-tar + recompress to a tmp file and
//!      atomically rename over `graph.tar.zst`.
//!
//! Fingerprints (file → blake3 hash), layers, tour, and embedding
//! meta live in `meta.json` inside the archive.
//! `.gitignore` / `.understandignore` filtering is in [`ignore_filter`].

pub mod archive;
pub mod fingerprint;
pub mod gitignore;
pub mod ignore_filter;
pub mod layout;
pub mod staleness;
pub mod storage;

pub use archive::{UNDERSTANDABLE_SCHEMA_VERSION, ZSTD_LEVEL};
pub use fingerprint::{blake3_file, blake3_string, Fingerprint};
pub use gitignore::{apply_block, render_block, GitignoreOutcome, GitignorePolicy};
pub use ignore_filter::{walk_project, IgnoreFilter};
pub use layout::ProjectLayout;
pub use staleness::{StalenessReport, StalenessStatus};
pub use storage::{
    uuid_for_key, EmbeddingBatchRow, LlmCacheEntry, LlmCacheKey, LlmOutputCache, Storage, VectorHit,
};

/// Re-export the workspace-wide error so callers don't have to depend
/// on `ua_core` directly for it.
pub use ua_core::Error;

/// Deprecated alias for [`ua_core::Error`]. Old callers that imported
/// `ua_persist::StorageError` keep compiling.
#[deprecated(
    since = "0.2.0",
    note = "use `ua_persist::Error` (re-exported from `ua_core`)"
)]
pub type StorageError = ua_core::Error;
