//! `Storage` — open / save the in-memory IndraDB graph + usearch ANN
//! index, persisted as a single `tar.zst` archive.
//!
//! The public API is the same set of `async` methods the rest of the
//! workspace already depends on. Every mutation is buffered in memory;
//! `Storage::save` / `Storage::save_kind` packs everything into the
//! archive atomically (tmp + fsync + rename + parent fsync).
//!
//! See [`crate::archive`] for the on-disk layout.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use indradb::{
    AllEdgeQuery, AllVertexQuery, BulkInsertItem, Database, Edge, Identifier, MemoryDatastore,
    Query as IndraQuery, QueryExt, QueryOutputValue, SpecificVertexQuery,
};

use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use ua_core::{
    EdgeDirection, EdgeType, Error, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, Layer,
    NodeType, ProjectMeta, TourStep,
};

use crate::archive::{
    self, entry, ArchiveEntry, UNDERSTANDABLE_SCHEMA_VERSION,
};
use crate::fingerprint::Fingerprint;
use crate::layout::ProjectLayout;

// ---- public surface --------------------------------------------------------

/// Search hit returned by [`Storage::vector_top_k`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VectorHit {
    pub node_id: String,
    pub distance: f32,
}

/// One row in a bulk embedding upsert: `(node_id, text_hash, vector)`.
pub type EmbeddingBatchRow<'a> = (&'a str, &'a str, &'a [f32]);

/// Composite key for the per-file LLM output cache.
///
/// Two halves: `node_id` (e.g. `file:src/auth.rs`) and `prompt_hash`,
/// the blake3 of `format!("{system}|{user}")`. Binding the cache to
/// the prompt template means a future prompt change naturally
/// invalidates every entry — the lookup misses and the LLM gets
/// re-asked.
#[derive(Debug, Hash, PartialEq, Eq, Serialize, Deserialize, Clone)]
pub struct LlmCacheKey {
    pub node_id: String,
    pub prompt_hash: String,
}

/// One cached LLM response. The `file_hash` is the blake3 of the file
/// body the LLM saw when this entry was produced; lookups that pass a
/// different hash MUST treat the entry as a miss (the file changed).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmCacheEntry {
    /// blake3 of the file body the LLM saw. Used to invalidate the
    /// entry the moment the file changes on disk.
    pub file_hash: String,
    /// Raw response text — caller decides parsing. We do *not* try to
    /// understand the schema here so callers can change the prompt
    /// shape without churning the persistence layer.
    pub response: String,
    /// Unix seconds when the entry was written. Informational only —
    /// the cache is invalidated by hash mismatches, not by age.
    pub created_at: i64,
}

/// On-disk version of [`LlmOutputCache`]. Bumped whenever the struct
/// shape changes incompatibly; the decoder fails fast on mismatch.
const LLM_OUTPUT_CACHE_VERSION: u32 = 1;

/// Per-file LLM response cache, persisted as `llm_outputs.bincode`
/// inside the project archive.
///
/// The map is intentionally `HashMap` not `BTreeMap`: the access
/// pattern is point lookups during analyze, not iteration. Insertion
/// order is irrelevant since bincode serialises the whole map at save
/// time.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmOutputCache {
    /// On-disk version stamp. Defaults to
    /// [`LLM_OUTPUT_CACHE_VERSION`]; bump it whenever the struct
    /// shape changes incompatibly. Decoder rejects mismatches with
    /// [`Error::Schema`].
    #[serde(default = "default_llm_output_cache_version")]
    pub version: u32,
    /// Keyed by [`LlmCacheKey`] (`node_id` + `prompt_hash`). Value
    /// records the file_hash that was current when this output was
    /// produced + the response text. A miss (or a hit whose
    /// `file_hash` doesn't match the current file) means re-run the
    /// LLM.
    pub entries: HashMap<LlmCacheKey, LlmCacheEntry>,
}

fn default_llm_output_cache_version() -> u32 {
    LLM_OUTPUT_CACHE_VERSION
}

impl Default for LlmOutputCache {
    fn default() -> Self {
        Self {
            version: LLM_OUTPUT_CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

// ---- internal state --------------------------------------------------------

/// Deterministic UUID v5 namespace for business keys. Picked once and
/// frozen — changing it would re-key every persisted graph.
const UA_UUID_NAMESPACE: Uuid =
    Uuid::from_u128(0xb3b8d6f1_0000_0000_a000_000000000000);

pub fn uuid_for_key(key: &str) -> Uuid {
    Uuid::new_v5(&UA_UUID_NAMESPACE, key.as_bytes())
}

/// Sanitize a snake_case-ish string into a valid IndraDB identifier
/// (`[A-Za-z0-9_-]{1,255}`). The application taxonomy is already in
/// the right shape, but we run it through this filter as a safety net.
fn sanitize_identifier(raw: &str) -> Identifier {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    if out.len() > 255 {
        out.truncate(255);
    }
    // SAFETY: every byte is ascii alnum / `-` / `_`; len <= 255.
    unsafe { Identifier::new_unchecked(out) }
}

// ---- archive payload structs ----------------------------------------------

/// Side-car JSON written into `meta.json` of the archive. Carries
/// everything that doesn't belong inside IndraDB's internal struct
/// (fingerprints, layers, tour, embedding meta, project root stamp).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ArchiveMeta {
    schema_version: u32,
    project_root: Option<String>,
    project: ProjectMeta,
    graph_version: String,
    graph_kind: Option<String>,
    fingerprints: Vec<FingerprintRow>,
    layers: Vec<Layer>,
    tour: Vec<TourStep>,
    /// embedding meta keyed by model name.
    embedding_meta: HashMap<String, EmbeddingMetaRow>,
    /// number of vertices / edges; informational only.
    vertex_count: u64,
    edge_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct EmbeddingMetaRow {
    dim: usize,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FingerprintRow {
    path: String,
    hash: String,
    modified_at: Option<i64>,
    /// Tree-sitter / parser-derived structural hash. `#[serde(default)]`
    /// keeps legacy archives readable: a row written before this field
    /// existed deserialises with `None` here, and the change classifier
    /// transparently falls back to its byte-level heuristics until the
    /// next analyze run repopulates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    structural_hash: Option<String>,
}

impl From<&Fingerprint> for FingerprintRow {
    fn from(f: &Fingerprint) -> Self {
        Self {
            path: f.path.clone(),
            hash: f.hash.clone(),
            modified_at: f.modified_at,
            structural_hash: f.structural_hash.clone(),
        }
    }
}

impl From<FingerprintRow> for Fingerprint {
    fn from(f: FingerprintRow) -> Self {
        Self {
            path: f.path,
            hash: f.hash,
            modified_at: f.modified_at,
            structural_hash: f.structural_hash,
        }
    }
}

/// On-disk version of `embeddings.bin`. Bumped whenever the body
/// layout changes; decoders must match exactly. To migrate, replace
/// the equality check in [`decode_embeddings`] with a
/// `match header.version { 1 => …, 2 => …, _ => Error::Schema }`
/// branch and translate the older payload into the current shape.
const EMBEDDINGS_HEADER_VERSION: u32 = 1;

/// Header for `embeddings.bin`. Wraps the raw `Vec<f32>` payload with
/// enough metadata that we can rebuild the usearch ANN index without
/// the `vectors.usearch` blob — and survive future ANN crate bumps
/// without breaking previously persisted archives.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct EmbeddingsHeader {
    /// Bumped if the body layout changes. Decoder fails fast on
    /// mismatch ([`EMBEDDINGS_HEADER_VERSION`]).
    version: u32,
    /// Embedding rows in insertion order.
    rows: Vec<EmbeddingsHeaderRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbeddingsHeaderRow {
    node_id: String,
    model: String,
    dim: usize,
    text_hash: String,
    updated_at: i64,
}

// ---- in-memory mutable state -----------------------------------------------

/// Embedding row stored in memory. The position in
/// [`EmbeddingsState::rows`] doubles as the `u64` key handed to
/// usearch — strings aren't stored inside the index, so we keep a
/// parallel lookup vec.
#[derive(Debug, Clone)]
struct EmbeddingRow {
    node_id: String,
    model: String,
    vector: Vec<f32>,
    text_hash: String,
    updated_at: i64,
}

/// Holds either a freshly built / reloaded index or an mmap-only view
/// pulled in from a saved `vectors.usearch` blob during `from_archive`.
/// The view is *read-only*: the first mutation drops it and the next
/// `vector_top_k` rebuilds from `rows` end-to-end.
#[derive(Default)]
enum IndexState {
    #[default]
    None,
    /// Live index built in-process (mutable, query-able).
    Built {
        index: Index,
        /// Model whose rows the index was built from.
        model: String,
    },
    /// Mmap'd cold-open view. `model` is the model we believe the
    /// view answers for; if a future query asks for a different model
    /// we drop the view and rebuild.
    View {
        index: Index,
        model: String,
    },
}

/// Above this size, a cold-open dump is spilled to a tempfile
/// immediately rather than carrying the bytes in `pending_views` until
/// the first query. Keeping multi-hundred-MB indices in RAM until
/// query time would dwarf the rest of the process; below the cap,
/// holding the bytes pays off the no-vector-workflow scenario where we
/// never query at all.
const MAX_LAZY_VIEW_BYTES: usize = 64 * 1024 * 1024;

/// One pending cold-open dump waiting to be installed as a usearch
/// `Index::view`. Two flavours:
///
/// * `Bytes` — the dump is small enough that we keep it in RAM and
///   only spill to a tempfile when [`EmbeddingsState::try_install_view_for`]
///   actually fires. No-vector workflows pay zero filesystem I/O.
/// * `Spilled` — the dump exceeded [`MAX_LAZY_VIEW_BYTES`] at open
///   time and was written to a tempfile up front. The tempfile must
///   outlive the resulting `Index::view` mmap, so we hold it here.
enum PendingView {
    Bytes(Vec<u8>),
    Spilled {
        // tempfile must live as long as the mmap'd view; once
        // installed it moves into `EmbeddingsState::live_view_temps`,
        // but until then it sits here.
        _tmp: tempfile::NamedTempFile,
        path: PathBuf,
    },
}

#[derive(Default)]
struct EmbeddingsState {
    /// Stable insertion-ordered list — the row position is the u64
    /// key we hand to usearch.
    rows: Vec<EmbeddingRow>,
    /// Per-(node_id, model) index into `rows`. Last write wins.
    by_key: HashMap<(String, String), usize>,
    /// Per-model dimension (from `ensure_embeddings_table`).
    dims: HashMap<String, EmbeddingMetaRow>,
    /// Lazy ANN index. `None` until first `vector_top_k`, rebuilt
    /// whenever `index_dirty` is `true`. May also be a `View` mmap'd
    /// from disk for cold-open (see [`IndexState::View`]).
    index: IndexState,
    index_dirty: bool,
    /// Per-model lazy cold-open dumps extracted from the last opened
    /// archive (also accepts the legacy single-blob `vectors.usearch`
    /// written by older saves of a sole-model archive — that entry
    /// is keyed under whichever model `sole_model` resolved to during
    /// `from_archive`).
    ///
    /// Variants ([`PendingView`]):
    ///   * `Bytes` — dump kept in RAM until first query (small
    ///     dumps; saves the tempfile write for no-vector workflows).
    ///   * `Spilled` — dump exceeded [`MAX_LAZY_VIEW_BYTES`] and was
    ///     materialised to a tempfile at open time.
    ///
    /// Each entry survives until either:
    ///   * the first `vector_top_k(model)` consumes it via
    ///     [`Self::try_install_view_for`] (one-shot — entry removed on
    ///     install, regardless of success), or
    ///   * a mutation calls [`Self::invalidate_index`] which clears the
    ///     whole map (any dump no longer matches the live rows).
    ///
    /// Holding entries *per model* — not a single
    /// `Option<PendingView>` — is what unlocks multi-model cold-open:
    /// each model's first query pays only its own mmap install,
    /// never the rebuild.
    pending_views: HashMap<String, PendingView>,
    /// Tempfiles backing live `Index::view` mmaps. A `view` mmap
    /// references the file's bytes via the OS page cache, so the
    /// tempfile must outlive the `Index::view`. Both immediate-spill
    /// (large blob from `from_archive`) and lazy-materialise (small
    /// blob promoted at first query) deposits land here. Dropped
    /// when `EmbeddingsState` itself drops, which happens with
    /// `Storage`.
    live_view_temps: Vec<tempfile::NamedTempFile>,
}

impl EmbeddingsState {
    fn dim_for(&self, model: &str) -> Option<usize> {
        self.dims.get(model).map(|m| m.dim)
    }

    fn count_for(&self, model: &str) -> u64 {
        self.rows.iter().filter(|r| r.model == model).count() as u64
    }

    fn upsert(
        &mut self,
        node_id: &str,
        model: &str,
        vector: &[f32],
        text_hash: &str,
        now: i64,
    ) -> Result<(), Error> {
        if let Some(dim) = self.dim_for(model) {
            if vector.len() != dim {
                return Err(Error::EmbeddingDimMismatch {
                    model: model.to_string(),
                    stored: dim,
                    new: vector.len(),
                });
            }
        }
        let key = (node_id.to_string(), model.to_string());
        if let Some(&idx) = self.by_key.get(&key) {
            self.rows[idx].vector = vector.to_vec();
            self.rows[idx].text_hash = text_hash.to_string();
            self.rows[idx].updated_at = now;
        } else {
            let idx = self.rows.len();
            self.rows.push(EmbeddingRow {
                node_id: node_id.to_string(),
                model: model.to_string(),
                vector: vector.to_vec(),
                text_hash: text_hash.to_string(),
                updated_at: now,
            });
            self.by_key.insert(key, idx);
        }
        self.invalidate_index();
        Ok(())
    }

    fn forget(&mut self, node_ids: &[String]) {
        if node_ids.is_empty() {
            return;
        }
        let drop_ids: std::collections::HashSet<&str> =
            node_ids.iter().map(|s| s.as_str()).collect();
        // Rebuild rows + by_key in order, skipping the deletions.
        let mut new_rows: Vec<EmbeddingRow> = Vec::with_capacity(self.rows.len());
        let mut new_by_key: HashMap<(String, String), usize> = HashMap::new();
        for row in self.rows.drain(..) {
            if drop_ids.contains(row.node_id.as_str()) {
                continue;
            }
            let idx = new_rows.len();
            new_by_key.insert((row.node_id.clone(), row.model.clone()), idx);
            new_rows.push(row);
        }
        self.rows = new_rows;
        self.by_key = new_by_key;
        self.invalidate_index();
    }

    fn reset_model(&mut self, model: &str) {
        let mut new_rows: Vec<EmbeddingRow> = Vec::with_capacity(self.rows.len());
        let mut new_by_key: HashMap<(String, String), usize> = HashMap::new();
        for row in self.rows.drain(..) {
            if row.model == model {
                continue;
            }
            let idx = new_rows.len();
            new_by_key.insert((row.node_id.clone(), row.model.clone()), idx);
            new_rows.push(row);
        }
        self.rows = new_rows;
        self.by_key = new_by_key;
        self.dims.remove(model);
        self.invalidate_index();
    }

    /// Drop the live index / view and mark dirty. Every entry in
    /// `pending_views` is also discarded — the on-disk dumps no longer
    /// match the live rows, and we'd rather pay a rebuild on the next
    /// query than serve stale results.
    fn invalidate_index(&mut self) {
        if !matches!(self.index, IndexState::None) {
            tracing::debug!("dropping stale ANN index after mutation");
        }
        self.index = IndexState::None;
        self.pending_views.clear();
        self.index_dirty = true;
    }

    /// Try to mmap the cold-open dump into an `Index::view`. Only fires
    /// when no mutations happened since open *and* a per-model dump
    /// entry is registered for `model` *and* the dim is known. Returns
    /// `true` if the view was installed.
    ///
    /// One-shot: the entry is removed from `pending_views` on every
    /// path (success or failure) so we don't keep retrying a dump that
    /// fails to load.
    ///
    /// For [`PendingView::Bytes`] entries this is also where lazy
    /// materialisation happens — we write the bytes to a fresh
    /// tempfile, mmap it, and stash the tempfile in
    /// [`Self::live_view_temps`] so the mmap stays valid for the
    /// remainder of `Storage`'s lifetime. Bytes are dropped on every
    /// path so we never hold both copies.
    fn try_install_view_for(&mut self, model: &str) -> bool {
        let Some(pending) = self.pending_views.remove(model) else {
            return false;
        };
        let Some(dim) = self.dim_for(model) else {
            return false;
        };
        // Materialise to a tempfile if we were holding bytes. Either way
        // we end up with `(tmp, path)`; `tmp` keeps the file alive long
        // enough for the mmap, and we hand ownership over to
        // `live_view_temps` on success.
        let (tmp, path) = match pending {
            PendingView::Bytes(bytes) => match stash_view_blob(&bytes) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(error = %e, %model, "lazy view materialise failed");
                    return false;
                }
            },
            PendingView::Spilled { _tmp, path } => (_tmp, path),
        };
        // Safety net: the dump may have been written by a prior schema
        // version we no longer trust. Build the IndexOptions to match
        // what we'd build for a fresh save.
        let options = build_index_options(dim);
        let index = match Index::new(&options) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(error = %e, "Index::new for view init failed");
                return false;
            }
        };
        let _ = dim; // dim is validated implicitly by Index::view loading the header.
        match index.view(path.to_string_lossy().as_ref()) {
            Ok(()) => {
                tracing::debug!(model = %model, "installed mmap'd usearch view for cold-open");
                self.live_view_temps.push(tmp);
                self.index = IndexState::View {
                    index,
                    model: model.to_string(),
                };
                self.index_dirty = false;
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "Index::view failed — will fall back to rebuild");
                false
            }
        }
    }

    /// Build (or rebuild) the usearch index over `model`'s rows.
    fn rebuild_index_for(&mut self, model: &str) -> Result<(), Error> {
        let model_rows: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.model == model)
            .map(|(i, _)| i)
            .collect();
        if model_rows.is_empty() {
            self.index = IndexState::None;
            self.index_dirty = false;
            return Ok(());
        }
        let dim = self.rows[model_rows[0]].vector.len();
        let options = build_index_options(dim);
        let index = Index::new(&options)
            .map_err(|e| Error::Hnsw(format!("usearch new: {e}")))?;
        index
            .reserve(model_rows.len().max(16))
            .map_err(|e| Error::Hnsw(format!("usearch reserve: {e}")))?;
        for &i in &model_rows {
            let row = &self.rows[i];
            index
                .add(i as u64, &row.vector)
                .map_err(|e| Error::Hnsw(format!("usearch add: {e}")))?;
        }
        tracing::debug!(rows = model_rows.len(), %model, "rebuilt usearch index");
        self.index = IndexState::Built {
            index,
            model: model.to_string(),
        };
        self.index_dirty = false;
        Ok(())
    }
}

/// Shared `IndexOptions` factory — single source of truth for `dim`,
/// metric and quantization so build / view / save all agree.
fn build_index_options(dim: usize) -> IndexOptions {
    IndexOptions {
        dimensions: dim,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        // 0 = let usearch pick sane defaults.
        connectivity: 0,
        expansion_add: 0,
        expansion_search: 0,
        ..Default::default()
    }
}

// ---- Storage ---------------------------------------------------------------

/// In-memory, archive-backed graph + vector store.
///
/// # Lock-acquisition order
///
/// `Storage` holds five independent locks. Any code path that grabs
/// more than one MUST acquire them in this canonical order to avoid
/// deadlocks (a parallel `save_kind` versus `save_graph` was
/// previously deadlock-prone exactly because of disagreeing orders):
///
/// 1. `id_map`
/// 2. `meta`
/// 3. `embeddings`
/// 4. `llm_cache`
/// 5. `graph`
///
/// "Acquire" means *enter* the critical section; you may release in
/// any order. If a method only needs one lock it can grab it
/// directly; if it needs several, it must take them top-to-bottom.
/// Cross-method invariants (e.g. "build the graph msgpack while
/// holding nothing else") are documented in the methods themselves.
pub struct Storage {
    /// IndraDB in-memory database. Sync — wrapped in a mutex so the
    /// async API can hold it across `.await`s.
    graph: tokio::sync::Mutex<Database<MemoryDatastore>>,
    /// Business key (e.g. `file:src/auth.rs`) → vertex UUID. Derived
    /// from UUID v5 so it stays stable across rebuilds, but we cache
    /// it so callers don't pay the hash on every lookup.
    id_map: tokio::sync::RwLock<HashMap<String, Uuid>>,
    embeddings: tokio::sync::RwLock<EmbeddingsState>,
    /// Misc archive-level state — copied out of `meta.json` on open
    /// and rewritten whole on save.
    meta: tokio::sync::RwLock<ArchiveMeta>,
    /// Per-file LLM response cache. Persisted as `llm_outputs.bincode`
    /// inside the archive. Loaded on open (graceful: missing entry →
    /// empty), rewritten whole on save.
    llm_cache: tokio::sync::RwLock<LlmOutputCache>,
}

impl Storage {
    /// Open the storage backing the given project layout.
    pub async fn open(layout: &ProjectLayout) -> Result<Self, Error> {
        Self::open_kind(layout, "codebase").await
    }

    /// Open the per-kind store (`codebase` / `domain` / `knowledge`).
    pub async fn open_kind(layout: &ProjectLayout, kind: &str) -> Result<Self, Error> {
        let archive_path = layout.graph_archive_for(kind);
        let storage = Self::from_archive(&archive_path).await?;
        storage.check_project_root(layout).await?;
        Ok(storage)
    }

    /// Open in a brand-new in-memory backend with no persisted state.
    pub async fn open_fresh() -> Result<Self, Error> {
        Ok(Self::empty())
    }

    fn empty() -> Self {
        Self {
            graph: tokio::sync::Mutex::new(MemoryDatastore::new_db()),
            id_map: tokio::sync::RwLock::new(HashMap::new()),
            embeddings: tokio::sync::RwLock::new(EmbeddingsState::default()),
            meta: tokio::sync::RwLock::new(ArchiveMeta {
                schema_version: UNDERSTANDABLE_SCHEMA_VERSION,
                ..Default::default()
            }),
            llm_cache: tokio::sync::RwLock::new(LlmOutputCache::default()),
        }
    }

    /// Decompress + decode a `<dst>.tar.zst` into in-memory state.
    async fn from_archive(path: &std::path::Path) -> Result<Self, Error> {
        let entries = archive::read_archive(path)?;
        if entries.is_empty() {
            return Ok(Self::empty());
        }
        let mut meta: ArchiveMeta = ArchiveMeta {
            schema_version: UNDERSTANDABLE_SCHEMA_VERSION,
            ..Default::default()
        };
        let mut id_map: HashMap<String, Uuid> = HashMap::new();
        let mut graph_msgpack: Option<Vec<u8>> = None;
        let mut embeddings_bin: Option<Vec<u8>> = None;
        // Legacy single-blob dump (sole-model archives, all schemas).
        let mut legacy_vectors_usearch: Option<Vec<u8>> = None;
        // Per-model dumps (`vectors.<model>.usearch`). Multi-model
        // archives carry one entry per model; sole-model archives may
        // carry a single per-model entry instead of the legacy blob.
        let mut per_model_vectors: HashMap<String, Vec<u8>> = HashMap::new();
        // LLM output cache (optional). Missing → empty cache (older
        // archives didn't persist this).
        let mut llm_cache: LlmOutputCache = LlmOutputCache::default();

        for ArchiveEntry { name, bytes } in entries {
            match name.as_str() {
                entry::META_JSON => {
                    meta = serde_json::from_slice(&bytes)?;
                }
                entry::ID_MAP_BINCODE => {
                    id_map = bincode::deserialize(&bytes)
                        .map_err(|e| Error::Bincode(e.to_string()))?;
                }
                entry::GRAPH_MSGPACK => {
                    graph_msgpack = Some(bytes);
                }
                entry::EMBEDDINGS_BIN => {
                    embeddings_bin = Some(bytes);
                }
                entry::VECTORS_USEARCH => {
                    legacy_vectors_usearch = Some(bytes);
                }
                entry::LLM_OUTPUTS_BINCODE => {
                    // Tolerant decode for malformed bincode (a
                    // corrupted entry shouldn't block the rest of the
                    // archive — worst case: every file misses on the
                    // next analyze run). But version mismatches are
                    // explicit: a future writer's payload should not
                    // be silently dropped — surface the error so the
                    // user knows to upgrade the binary.
                    match bincode::deserialize::<LlmOutputCache>(&bytes) {
                        Ok(cache) => {
                            if cache.version != LLM_OUTPUT_CACHE_VERSION {
                                return Err(Error::Schema(format!(
                                    "llm_outputs.bincode version mismatch: got {}, expected {}",
                                    cache.version, LLM_OUTPUT_CACHE_VERSION
                                )));
                            }
                            llm_cache = cache;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "could not decode llm_outputs.bincode — continuing with empty cache"
                            );
                        }
                    }
                }
                entry::LEGACY_HNSW_GRAPH | entry::LEGACY_HNSW_DATA => {
                    tracing::debug!(%name, "ignoring legacy hnsw_rs entry — will rebuild");
                }
                other => {
                    // Per-model usearch dumps are written as
                    // `vectors.<model>.usearch`. Anything that fits
                    // that shape is captured here; everything else is
                    // genuinely unknown and warned on.
                    if let Some(model) = parse_per_model_vectors_entry(other) {
                        per_model_vectors.insert(model, bytes);
                    } else {
                        tracing::warn!(name = %other, "unknown archive entry — ignored");
                    }
                }
            }
        }

        if meta.schema_version > UNDERSTANDABLE_SCHEMA_VERSION {
            return Err(Error::Schema(format!(
                "archive schema version {} is newer than this binary (max {})",
                meta.schema_version, UNDERSTANDABLE_SCHEMA_VERSION
            )));
        }

        let db = match graph_msgpack {
            Some(bytes) => decode_msgpack_db(&bytes)?,
            None => MemoryDatastore::new_db(),
        };

        // Rebuild EmbeddingsState from `embeddings.bin`. The vectors
        // file is the *source of truth* — the per-model usearch dumps
        // are just ANN caches that let us mmap a cold-open query
        // without paying the rebuild cost. Multi-model archives carry
        // one dump per model (`vectors.<model>.usearch`); legacy
        // archives carry a single `vectors.usearch` blob keyed under
        // whichever model was registered at save time.
        let mut embeddings = EmbeddingsState {
            dims: meta.embedding_meta.clone(),
            ..Default::default()
        };
        if let Some(bytes) = embeddings_bin {
            decode_embeddings(&bytes, &mut embeddings)?;
            embeddings.index_dirty = true;
        }

        // 1. Per-model dumps. We try lazy materialisation first
        //    (keep bytes in RAM, spill on first query) — that way a
        //    no-vector workflow like `analyze --plan-only` pays zero
        //    filesystem I/O for these dumps. Anything bigger than
        //    [`MAX_LAZY_VIEW_BYTES`] is spilled immediately so we
        //    don't pin a huge blob in process memory.
        for (model, bytes) in per_model_vectors {
            embeddings
                .pending_views
                .insert(model.clone(), build_pending_view(bytes, &model));
        }

        // 2. Legacy single-blob dump — schema v3 always wrote this for
        //    sole-model archives, and we still write it for sole-model
        //    archives today (so this read path stays warm). Resolve
        //    the model via `sole_model`; if the archive turned out to
        //    have multiple models registered (no sole model) and a
        //    per-model entry already covers the same model, the
        //    per-model entry wins (insertion above happened first).
        if let Some(bytes) = legacy_vectors_usearch {
            match sole_model(&embeddings) {
                Some(model) if !embeddings.pending_views.contains_key(&model) => {
                    let entry = build_pending_view(bytes, &model);
                    embeddings.pending_views.insert(model, entry);
                }
                Some(_) => {
                    tracing::debug!(
                        "legacy vectors.usearch ignored — per-model dump already present"
                    );
                }
                None => {
                    tracing::debug!(
                        "legacy vectors.usearch present but no unique model — view skipped"
                    );
                }
            }
        }

        Ok(Self {
            graph: tokio::sync::Mutex::new(db),
            id_map: tokio::sync::RwLock::new(id_map),
            embeddings: tokio::sync::RwLock::new(embeddings),
            meta: tokio::sync::RwLock::new(meta),
            llm_cache: tokio::sync::RwLock::new(llm_cache),
        })
    }

    async fn check_project_root(&self, layout: &ProjectLayout) -> Result<(), Error> {
        let Some(current) = layout.project_root_stamp() else {
            return Ok(());
        };
        let mut meta = self.meta.write().await;
        match meta.project_root.as_deref() {
            None => {
                tracing::warn!(project_root = %current, "stamping project_root into legacy archive");
                meta.project_root = Some(current);
                Ok(())
            }
            Some(s) if s == current => Ok(()),
            Some("") => {
                meta.project_root = Some(current);
                Ok(())
            }
            Some(s) => Err(Error::ProjectRootMismatch {
                stored: s.to_string(),
                current,
            }),
        }
    }

    /// Persist the live state to `<layout>/<db_name>.tar.zst`.
    pub async fn save(&self, layout: &ProjectLayout) -> Result<(), Error> {
        self.save_kind(layout, "codebase").await
    }

    /// Persist the per-kind archive.
    ///
    /// Lock order (canonical, see [`Storage`] doc): `id_map` →
    /// `meta` → `embeddings` → `llm_cache` → `graph`. Each lock is
    /// taken in its own narrow scope so other callers can interleave
    /// — most notably `vector_top_k`, which only needs `embeddings`
    /// and gets no contention with the embeddings dump (we snapshot
    /// rows under a read lock and then drop it before dumping).
    pub async fn save_kind(&self, layout: &ProjectLayout, kind: &str) -> Result<(), Error> {
        layout.ensure_exists()?;
        let dst = layout.graph_archive_for(kind);

        // 1. Build the graph msgpack via IndraDB's
        //    `MemoryDatastore::create_msgpack_db` API. The crate only
        //    exposes a path-based dump; we point it at a tempfile,
        //    `sync` to flush, and read the bytes back. Re-encoding via
        //    `rmp_serde` directly isn't possible because
        //    `InternalMemory` is `pub(crate)`. Tracked upstream as
        //    "dump_to_writer" — we'd file an issue.
        //
        //    `serialize_graph_to_msgpack` takes the `graph` lock
        //    internally. We hold no other locks here so the canonical
        //    order is trivially honoured.
        let graph_msgpack = self.serialize_graph_to_msgpack().await?;

        // 2. Serialise id_map (bincode). Lock #1 in canonical order.
        let id_map_bytes = {
            let map = self.id_map.read().await;
            bincode::serialize(&*map).map_err(|e| Error::Bincode(e.to_string()))?
        };

        // 3. Snapshot the embeddings rows + dims under a *read* lock,
        //    then drop the lock immediately. The dump and the
        //    embeddings.bin encode both happen on the snapshot, so the
        //    dashboard can keep querying `vector_top_k` while we
        //    rebuild indices and write tempfiles. The trade-off: the
        //    on-disk dump lags the live state by however long the
        //    snapshot took; the next `save_kind` picks up the drift.
        //
        //    Critically we no longer call `rebuild_index_for` on the
        //    live state during dump — the previous code held a *write*
        //    lock for the entire dump phase, which blocked every
        //    concurrent query. The new flow doesn't mutate `self`'s
        //    embeddings at all.
        let (rows_snapshot, dims_snapshot) = {
            let st = self.embeddings.read().await;
            (st.rows.clone(), st.dims.clone())
        };
        let embeddings_bin = encode_embeddings_from_snapshot(&rows_snapshot)?;

        // 4. usearch dumps from the snapshot. Sole-model archives
        //    keep writing the legacy `vectors.usearch` entry
        //    (preserves cold-open for older readers and avoids
        //    churning the on-disk layout for the common case).
        //    Multi-model archives emit one `vectors.<model>.usearch`
        //    entry per model so each model's first query can lazily
        //    mmap its own dump instead of paying a full rebuild.
        //
        //    Both forms are read-tolerated by `from_archive` — the
        //    write path's choice is purely about backwards
        //    compatibility with existing archives and tests.
        enum DumpEntries {
            None,
            Legacy(Vec<u8>),
            PerModel(Vec<(String, Vec<u8>)>),
        }
        let models = registered_models_from_snapshot(&rows_snapshot, &dims_snapshot);
        let dump_entries: DumpEntries = match models.len() {
            0 => DumpEntries::None,
            1 => {
                let model = models.into_iter().next().expect("len==1");
                match build_dump_from_rows(&rows_snapshot, &model) {
                    Ok(bytes) => DumpEntries::Legacy(bytes),
                    Err(e) => {
                        tracing::warn!(error = %e, %model, "skipping vectors.usearch dump");
                        DumpEntries::None
                    }
                }
            }
            _ => {
                let mut out: Vec<(String, Vec<u8>)> = Vec::with_capacity(models.len());
                for model in models {
                    match build_dump_from_rows(&rows_snapshot, &model) {
                        Ok(bytes) => out.push((model, bytes)),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                %model,
                                "skipping per-model usearch dump"
                            );
                        }
                    }
                }
                DumpEntries::PerModel(out)
            }
        };
        // Drop the snapshot now — meta / graph counters don't need it,
        // and freeing it keeps peak memory bounded.
        drop(rows_snapshot);

        // 5. Refresh meta counters / project root stamp. Canonical
        //    order requires `meta` *before* `graph`. We take meta
        //    first, copy `dims_snapshot` into it (no embeddings lock
        //    needed — we already snapshotted), then briefly grab the
        //    graph lock for the vertex/edge counts.
        let meta_bytes = {
            let mut m = self.meta.write().await;
            m.schema_version = UNDERSTANDABLE_SCHEMA_VERSION;
            m.embedding_meta = dims_snapshot;
            let db = self.graph.lock().await;
            m.vertex_count = count_query(&db, AllVertexQuery)? as u64;
            m.edge_count = count_query(&db, AllEdgeQuery)? as u64;
            drop(db);
            if let Some(root) = layout.project_root_stamp() {
                m.project_root = Some(root);
            }
            serde_json::to_vec(&*m)?
        };

        // 6. Snapshot the LLM output cache. Always serialise — even
        //    if empty — so future opens have a stable entry to read.
        //    The bincode payload is tiny for empty caches (a few
        //    bytes) and keeps the archive layout deterministic.
        //    `from_archive`'s "unknown archive entry — ignored" branch
        //    means older readers (predating
        //    [`entry::LLM_OUTPUTS_BINCODE`]) silently skip this entry
        //    rather than failing — there is no compat hazard in
        //    always emitting it.
        let llm_cache_bytes = {
            let cache = self.llm_cache.read().await;
            bincode::serialize(&*cache).map_err(|e| Error::Bincode(e.to_string()))?
        };

        let mut entries: Vec<(String, Vec<u8>)> = vec![
            (entry::META_JSON.to_string(), meta_bytes),
            (entry::ID_MAP_BINCODE.to_string(), id_map_bytes),
            (entry::GRAPH_MSGPACK.to_string(), graph_msgpack),
            (entry::EMBEDDINGS_BIN.to_string(), embeddings_bin),
            (entry::LLM_OUTPUTS_BINCODE.to_string(), llm_cache_bytes),
        ];
        match dump_entries {
            DumpEntries::None => {}
            DumpEntries::Legacy(bytes) => {
                entries.push((entry::VECTORS_USEARCH.to_string(), bytes));
            }
            DumpEntries::PerModel(per_model) => {
                for (model, bytes) in per_model {
                    entries.push((per_model_vectors_entry_name(&model), bytes));
                }
            }
        }
        archive::write_archive(&dst, entries)?;
        Ok(())
    }

    async fn serialize_graph_to_msgpack(&self) -> Result<Vec<u8>, Error> {
        // Open a tempfile, build a fresh persisted MemoryDatastore at
        // its path, copy every vertex / edge / property over, sync.
        // The original DB's `path` is None so it can't sync directly.
        let tmp = tempfile::Builder::new()
            .prefix("ua-graph-")
            .suffix(".msgpack")
            .tempfile()?;
        let path = tmp.path().to_path_buf();
        // `tmp` keeps the file alive for our re-read.
        let target = MemoryDatastore::create_msgpack_db(&path);

        // Mirror the live DB into `target`.
        let live = self.graph.lock().await;
        // Vertices.
        let vertices = match live
            .get(AllVertexQuery)
            .map_err(|e| Error::Graph(format!("dump vertices: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Vertices(v)) => v,
            _ => Vec::new(),
        };
        for v in &vertices {
            let _ = target.create_vertex(v);
        }
        // Vertex properties.
        for v in &vertices {
            let props = match live
                .get(SpecificVertexQuery::single(v.id).properties().map_err(|e| {
                    Error::Graph(format!("dump vertex properties: {e}"))
                })?)
                .map_err(|e| Error::Graph(format!("dump vertex properties: {e}")))?
                .pop()
            {
                Some(QueryOutputValue::VertexProperties(p)) => p,
                _ => Vec::new(),
            };
            for vp in props {
                for prop in vp.props {
                    target
                        .set_properties(SpecificVertexQuery::single(v.id), prop.name, &prop.value)
                        .map_err(|e| Error::Graph(format!("set vertex prop: {e}")))?;
                }
            }
        }
        // Edges.
        let edges = match live
            .get(AllEdgeQuery)
            .map_err(|e| Error::Graph(format!("dump edges: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Edges(e)) => e,
            _ => Vec::new(),
        };
        for e in &edges {
            let _ = target.create_edge(e);
        }
        // Edge properties — fetch each edge's props individually.
        for e in &edges {
            let q: IndraQuery = indradb::SpecificEdgeQuery::single(e.clone())
                .properties()
                .map_err(|e| Error::Graph(format!("dump edge prop query: {e}")))?
                .into();
            let props = match live
                .get(q)
                .map_err(|e| Error::Graph(format!("dump edge prop: {e}")))?
                .pop()
            {
                Some(QueryOutputValue::EdgeProperties(p)) => p,
                _ => Vec::new(),
            };
            for ep in props {
                for prop in ep.props {
                    target
                        .set_properties(
                            indradb::SpecificEdgeQuery::single(ep.edge.clone()),
                            prop.name,
                            &prop.value,
                        )
                        .map_err(|err| Error::Graph(format!("set edge prop: {err}")))?;
                }
            }
        }
        drop(live);

        target
            .sync()
            .map_err(|e| Error::Graph(format!("msgpack sync: {e}")))?;
        let bytes = std::fs::read(&path)?;
        Ok(bytes)
    }

    /// Wipe every vertex / edge / layer / tour entry and the project
    /// meta. Embeddings and fingerprints survive — call
    /// [`Self::reset_embeddings`] / [`Self::write_fingerprints`] for
    /// those.
    ///
    /// Lock order: `id_map` → `meta` → `graph`, matching the
    /// canonical order documented on [`Storage`].
    pub async fn clear_graph(&self) -> Result<(), Error> {
        let mut id_map = self.id_map.write().await;
        let mut meta = self.meta.write().await;
        let mut live = self.graph.lock().await;
        // Replace the live datastore wholesale; cheaper than walking
        // every vertex.
        *live = MemoryDatastore::new_db();
        id_map.clear();
        meta.layers.clear();
        meta.tour.clear();
        meta.project = ProjectMeta::default();
        meta.graph_version.clear();
        meta.graph_kind = None;
        meta.vertex_count = 0;
        meta.edge_count = 0;
        Ok(())
    }

    /// Replace the graph with `graph`. Layers, tour, project meta and
    /// graph kind go into the side-car `meta.json` because they're
    /// trivially rebuildable and don't need first-class graph nodes.
    ///
    /// Lock order: `id_map` → `meta` → `graph` (canonical, see
    /// [`Storage`]). The pre-existing layout took `meta` *after*
    /// `graph`, which deadlocks against any caller that takes `meta`
    /// first (e.g. `save_kind`); the fix is to acquire all three up
    /// front and hold them across the bulk insert.
    pub async fn save_graph(&self, graph: &KnowledgeGraph) -> Result<(), Error> {
        self.clear_graph().await?;

        // Build all ids first so `id_map` is populated before edges look
        // them up. `BulkInsertItem::Vertex` requires the UUID; the id
        // also has to be deterministic (UUID v5 of the business key).
        let mut id_map = self.id_map.write().await;
        let mut items: Vec<BulkInsertItem> = Vec::with_capacity(
            graph.nodes.len() * 6 + graph.edges.len() * 3,
        );
        let key_id_prop = unsafe { Identifier::new_unchecked("node_key") };
        let name_prop = unsafe { Identifier::new_unchecked("name") };
        let name_lower_prop = unsafe { Identifier::new_unchecked("name_lower") };
        let summary_prop = unsafe { Identifier::new_unchecked("summary") };
        let summary_lower_prop = unsafe { Identifier::new_unchecked("summary_lower") };
        let tags_prop = unsafe { Identifier::new_unchecked("tags") };
        let tags_text_prop = unsafe { Identifier::new_unchecked("tags_text") };
        let file_path_prop = unsafe { Identifier::new_unchecked("file_path") };
        let line_start_prop = unsafe { Identifier::new_unchecked("line_start") };
        let line_end_prop = unsafe { Identifier::new_unchecked("line_end") };
        let complexity_prop = unsafe { Identifier::new_unchecked("complexity") };
        let language_notes_prop = unsafe { Identifier::new_unchecked("language_notes") };
        let domain_meta_prop = unsafe { Identifier::new_unchecked("domain_meta") };
        let knowledge_meta_prop = unsafe { Identifier::new_unchecked("knowledge_meta") };
        let edge_direction_prop = unsafe { Identifier::new_unchecked("direction") };
        let edge_weight_prop = unsafe { Identifier::new_unchecked("weight") };
        let edge_description_prop = unsafe { Identifier::new_unchecked("description") };

        for n in &graph.nodes {
            let id = uuid_for_key(&n.id);
            id_map.insert(n.id.clone(), id);
            let t = sanitize_identifier(node_type_str(n.node_type));
            items.push(BulkInsertItem::Vertex(indradb::Vertex::with_id(id, t)));
            items.push(BulkInsertItem::VertexProperty(
                id,
                key_id_prop,
                indradb::Json::new(serde_json::Value::String(n.id.clone())),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                name_prop,
                indradb::Json::new(serde_json::Value::String(n.name.clone())),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                name_lower_prop,
                indradb::Json::new(serde_json::Value::String(n.name.to_lowercase())),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                summary_prop,
                indradb::Json::new(serde_json::Value::String(n.summary.clone())),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                summary_lower_prop,
                indradb::Json::new(serde_json::Value::String(n.summary.to_lowercase())),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                tags_prop,
                indradb::Json::new(serde_json::to_value(&n.tags)?),
            ));
            items.push(BulkInsertItem::VertexProperty(
                id,
                tags_text_prop,
                indradb::Json::new(serde_json::Value::String(n.tags.join(" ").to_lowercase())),
            ));
            if let Some(fp) = &n.file_path {
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    file_path_prop,
                    indradb::Json::new(serde_json::Value::String(fp.clone())),
                ));
            }
            if let Some((s, e)) = n.line_range {
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    line_start_prop,
                    indradb::Json::new(serde_json::Value::Number(s.into())),
                ));
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    line_end_prop,
                    indradb::Json::new(serde_json::Value::Number(e.into())),
                ));
            }
            items.push(BulkInsertItem::VertexProperty(
                id,
                complexity_prop,
                indradb::Json::new(serde_json::Value::String(
                    complexity_str(n.complexity).to_string(),
                )),
            ));
            if let Some(ln) = &n.language_notes {
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    language_notes_prop,
                    indradb::Json::new(serde_json::Value::String(ln.clone())),
                ));
            }
            if let Some(dm) = &n.domain_meta {
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    domain_meta_prop,
                    indradb::Json::new(serde_json::to_value(dm)?),
                ));
            }
            if let Some(km) = &n.knowledge_meta {
                items.push(BulkInsertItem::VertexProperty(
                    id,
                    knowledge_meta_prop,
                    indradb::Json::new(serde_json::to_value(km)?),
                ));
            }
        }

        for e in &graph.edges {
            let Some(&src) = id_map.get(&e.source) else {
                tracing::warn!(source = %e.source, "edge references missing source — skipped");
                continue;
            };
            let Some(&dst) = id_map.get(&e.target) else {
                tracing::warn!(target = %e.target, "edge references missing target — skipped");
                continue;
            };
            let t = sanitize_identifier(edge_type_str(e.edge_type));
            let edge = Edge::new(src, t, dst);
            items.push(BulkInsertItem::Edge(edge.clone()));
            items.push(BulkInsertItem::EdgeProperty(
                edge.clone(),
                edge_direction_prop,
                indradb::Json::new(serde_json::Value::String(
                    edge_direction_str(e.direction).to_string(),
                )),
            ));
            items.push(BulkInsertItem::EdgeProperty(
                edge.clone(),
                edge_weight_prop,
                indradb::Json::new(serde_json::json!(e.weight as f64)),
            ));
            if let Some(desc) = &e.description {
                items.push(BulkInsertItem::EdgeProperty(
                    edge,
                    edge_description_prop,
                    indradb::Json::new(serde_json::Value::String(desc.clone())),
                ));
            }
        }

        // Take meta BEFORE graph (canonical order). Stash side-car
        // state first so a caller waiting on `meta` (e.g. `save_kind`)
        // never sees us holding `graph` while we wait for `meta`.
        let mut meta = self.meta.write().await;
        meta.layers = graph.layers.clone();
        meta.tour = graph.tour.clone();
        meta.project = graph.project.clone();
        meta.graph_version = graph.version.clone();
        meta.graph_kind = graph.kind.map(graph_kind_str).map(|s| s.to_string());
        drop(meta);

        let live = self.graph.lock().await;
        live.bulk_insert(items)
            .map_err(|e| Error::Graph(format!("bulk_insert: {e}")))?;

        // Index the lookup properties. With these in place, the
        // property-value queries in `search_nodes` are O(log n) instead
        // of a full scan.
        live.index_property(name_lower_prop)
            .map_err(|e| Error::Graph(format!("index name_lower: {e}")))?;
        live.index_property(summary_lower_prop)
            .map_err(|e| Error::Graph(format!("index summary_lower: {e}")))?;
        live.index_property(tags_text_prop)
            .map_err(|e| Error::Graph(format!("index tags_text: {e}")))?;
        live.index_property(key_id_prop)
            .map_err(|e| Error::Graph(format!("index node_key: {e}")))?;
        drop(live);
        drop(id_map);
        Ok(())
    }

    /// Stamp the project root in side-car meta.
    pub async fn stamp_project_root(&self, layout: &ProjectLayout) -> Result<(), Error> {
        if let Some(root) = layout.project_root_stamp() {
            let mut meta = self.meta.write().await;
            meta.project_root = Some(root);
        }
        Ok(())
    }

    /// `save_graph` + `stamp_project_root` in one call.
    pub async fn save_graph_for(
        &self,
        graph: &KnowledgeGraph,
        layout: &ProjectLayout,
    ) -> Result<(), Error> {
        self.save_graph(graph).await?;
        self.stamp_project_root(layout).await?;
        Ok(())
    }

    /// Rebuild a [`KnowledgeGraph`] from the live state.
    ///
    /// Lock order: `id_map` → `meta` → `graph` (canonical, see
    /// [`Storage`]). The previous layout took `graph` first and only
    /// later acquired `id_map` / `meta`, which would deadlock with
    /// callers that take any of those before `graph` (every save
    /// path).
    pub async fn load_graph(&self) -> Result<KnowledgeGraph, Error> {
        let id_map = self.id_map.read().await;
        let meta = self.meta.read().await;
        let live = self.graph.lock().await;
        let vertices = match live
            .get(AllVertexQuery)
            .map_err(|e| Error::Graph(format!("load vertices: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Vertices(v)) => v,
            _ => Vec::new(),
        };
        let mut nodes: Vec<GraphNode> = Vec::with_capacity(vertices.len());
        for v in &vertices {
            let props = match live
                .get(
                    SpecificVertexQuery::single(v.id)
                        .properties()
                        .map_err(|e| Error::Graph(format!("vertex props query: {e}")))?,
                )
                .map_err(|e| Error::Graph(format!("vertex props: {e}")))?
                .pop()
            {
                Some(QueryOutputValue::VertexProperties(p)) => p,
                _ => Vec::new(),
            };
            let mut prop_map: HashMap<String, serde_json::Value> = HashMap::new();
            for vp in props {
                for p in vp.props {
                    prop_map.insert(p.name.as_str().to_string(), (*p.value.0).clone());
                }
            }
            let node_type = parse_node_type(v.t.as_str()).unwrap_or(NodeType::File);
            let id = prop_map
                .get("node_key")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.id.to_string());
            let name = prop_map
                .get("name")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let file_path = prop_map
                .get("file_path")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let summary = prop_map
                .get("summary")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let tags: Vec<String> = prop_map
                .get("tags")
                .and_then(|x| serde_json::from_value(x.clone()).ok())
                .unwrap_or_default();
            let complexity = prop_map
                .get("complexity")
                .and_then(|x| x.as_str())
                .map(parse_complexity)
                .unwrap_or(ua_core::Complexity::Moderate);
            let language_notes = prop_map
                .get("language_notes")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let line_range = match (
                prop_map.get("line_start").and_then(|x| x.as_u64()),
                prop_map.get("line_end").and_then(|x| x.as_u64()),
            ) {
                (Some(s), Some(e)) => Some((s as u32, e as u32)),
                _ => None,
            };
            let domain_meta = prop_map
                .get("domain_meta")
                .and_then(|x| serde_json::from_value(x.clone()).ok());
            let knowledge_meta = prop_map
                .get("knowledge_meta")
                .and_then(|x| serde_json::from_value(x.clone()).ok());
            nodes.push(GraphNode {
                id,
                node_type,
                name,
                file_path,
                line_range,
                summary,
                tags,
                complexity,
                language_notes,
                domain_meta,
                knowledge_meta,
            });
        }

        let edges_raw = match live
            .get(AllEdgeQuery)
            .map_err(|e| Error::Graph(format!("load edges: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Edges(e)) => e,
            _ => Vec::new(),
        };
        // Inverse id_map for edge lookups.
        let inverse: HashMap<Uuid, &str> = id_map.iter().map(|(k, v)| (*v, k.as_str())).collect();
        let mut edges: Vec<GraphEdge> = Vec::with_capacity(edges_raw.len());
        for e in edges_raw {
            let q: IndraQuery = indradb::SpecificEdgeQuery::single(e.clone())
                .properties()
                .map_err(|err| Error::Graph(format!("edge prop query: {err}")))?
                .into();
            let props = match live
                .get(q)
                .map_err(|err| Error::Graph(format!("edge prop: {err}")))?
                .pop()
            {
                Some(QueryOutputValue::EdgeProperties(p)) => p,
                _ => Vec::new(),
            };
            let mut prop_map: HashMap<String, serde_json::Value> = HashMap::new();
            for ep in props {
                for p in ep.props {
                    prop_map.insert(p.name.as_str().to_string(), (*p.value.0).clone());
                }
            }
            let edge_type = parse_edge_type(e.t.as_str()).unwrap_or(EdgeType::Related);
            let source = inverse
                .get(&e.outbound_id)
                .map(|s| s.to_string())
                .unwrap_or_else(|| e.outbound_id.to_string());
            let target = inverse
                .get(&e.inbound_id)
                .map(|s| s.to_string())
                .unwrap_or_else(|| e.inbound_id.to_string());
            let direction = prop_map
                .get("direction")
                .and_then(|x| x.as_str())
                .map(parse_direction)
                .unwrap_or(EdgeDirection::Forward);
            let weight = prop_map
                .get("weight")
                .and_then(|x| x.as_f64())
                .unwrap_or(1.0);
            let description = prop_map
                .get("description")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            edges.push(GraphEdge {
                source,
                target,
                edge_type,
                direction,
                weight: weight as f32,
                description,
            });
        }
        drop(live);
        drop(id_map);

        let result = KnowledgeGraph {
            version: if meta.graph_version.is_empty() {
                env!("CARGO_PKG_VERSION").to_string()
            } else {
                meta.graph_version.clone()
            },
            kind: meta.graph_kind.clone().and_then(|s| parse_graph_kind(&s)),
            project: meta.project.clone(),
            nodes,
            edges,
            layers: meta.layers.clone(),
            tour: meta.tour.clone(),
        };
        drop(meta);
        Ok(result)
    }

    /// Replace all fingerprints with `prints`.
    pub async fn write_fingerprints(&self, prints: &[Fingerprint]) -> Result<(), Error> {
        let mut meta = self.meta.write().await;
        meta.fingerprints = prints.iter().map(FingerprintRow::from).collect();
        Ok(())
    }

    pub async fn read_fingerprints(&self) -> Result<Vec<Fingerprint>, Error> {
        let meta = self.meta.read().await;
        Ok(meta
            .fingerprints
            .iter()
            .cloned()
            .map(Fingerprint::from)
            .collect())
    }

    /// Look up the recorded mtime (epoch seconds) for `path`.
    pub async fn file_modified_at(&self, path: &str) -> Result<Option<i64>, Error> {
        let meta = self.meta.read().await;
        Ok(meta
            .fingerprints
            .iter()
            .find(|f| f.path == path)
            .and_then(|f| f.modified_at))
    }

    /// Coarse prefilter for search. Tokenises `query` on whitespace and
    /// requires each token to appear in *some* indexable property
    /// (`name_lower` / `summary_lower` / `tags_text`). The downstream
    /// fuzzy ranker in [`ua_search`] re-orders the candidate set.
    pub async fn search_nodes(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, Error> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase())
            .collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let live = self.graph.lock().await;
        let vertices = match live
            .get(AllVertexQuery)
            .map_err(|e| Error::Graph(format!("search vertices: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Vertices(v)) => v,
            _ => Vec::new(),
        };
        let name_lower_prop = unsafe { Identifier::new_unchecked("name_lower") };
        let summary_lower_prop = unsafe { Identifier::new_unchecked("summary_lower") };
        let tags_text_prop = unsafe { Identifier::new_unchecked("tags_text") };
        let key_prop = unsafe { Identifier::new_unchecked("node_key") };
        let mut hits: Vec<String> = Vec::new();
        for v in &vertices {
            let mut name_l = String::new();
            let mut summary_l = String::new();
            let mut tags_t = String::new();
            let mut key = String::new();
            let q: IndraQuery = SpecificVertexQuery::single(v.id)
                .properties()
                .map_err(|e| Error::Graph(format!("search props query: {e}")))?
                .into();
            let props = match live
                .get(q)
                .map_err(|e| Error::Graph(format!("search props: {e}")))?
                .pop()
            {
                Some(QueryOutputValue::VertexProperties(p)) => p,
                _ => Vec::new(),
            };
            for vp in props {
                for p in vp.props {
                    if p.name == name_lower_prop {
                        if let Some(s) = p.value.as_str() {
                            name_l = s.to_string();
                        }
                    } else if p.name == summary_lower_prop {
                        if let Some(s) = p.value.as_str() {
                            summary_l = s.to_string();
                        }
                    } else if p.name == tags_text_prop {
                        if let Some(s) = p.value.as_str() {
                            tags_t = s.to_string();
                        }
                    } else if p.name == key_prop {
                        if let Some(s) = p.value.as_str() {
                            key = s.to_string();
                        }
                    }
                }
            }
            let all_match = tokens.iter().all(|t| {
                name_l.contains(t) || summary_l.contains(t) || tags_t.contains(t)
            });
            if all_match && !key.is_empty() {
                hits.push(key);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        Ok(hits)
    }

    /// `(target, edge_type)` pairs reachable from `node_id`.
    ///
    /// Lock order: `id_map` → `graph` (canonical, see [`Storage`]).
    pub async fn outgoing_edges(
        &self,
        node_id: &str,
    ) -> Result<Vec<(String, String)>, Error> {
        let id_map = self.id_map.read().await;
        let live = self.graph.lock().await;
        let Some(&src) = id_map.get(node_id) else {
            return Ok(Vec::new());
        };
        // Pipe vertex -> outbound edges.
        let inverse: HashMap<Uuid, String> =
            id_map.iter().map(|(k, v)| (*v, k.clone())).collect();
        let q: IndraQuery = SpecificVertexQuery::single(src)
            .outbound()
            .map_err(|e| Error::Graph(format!("outgoing query: {e}")))?
            .into();
        let edges = match live
            .get(q)
            .map_err(|e| Error::Graph(format!("outgoing edges: {e}")))?
            .pop()
        {
            Some(QueryOutputValue::Edges(e)) => e,
            _ => Vec::new(),
        };
        let mut out = Vec::with_capacity(edges.len());
        for e in edges {
            let target = inverse
                .get(&e.inbound_id)
                .cloned()
                .unwrap_or_else(|| e.inbound_id.to_string());
            out.push((target, e.t.as_str().to_string()));
        }
        Ok(out)
    }

    // ---- embeddings -------------------------------------------------------

    /// Register the embedding dimension for `model`. The new in-memory
    /// backend doesn't have a SQL "table" to create, so this method
    /// only records the `(model, dim)` mapping and rejects mismatching
    /// dims on subsequent calls.
    pub async fn ensure_embeddings_table(
        &self,
        model: &str,
        dim: usize,
    ) -> Result<(), Error> {
        let mut st = self.embeddings.write().await;
        if let Some(existing) = st.dims.get(model) {
            if existing.dim != dim {
                return Err(Error::EmbeddingDimMismatch {
                    model: model.to_string(),
                    stored: existing.dim,
                    new: dim,
                });
            }
            return Ok(());
        }
        // Cross-check the live rows: if any prior model row was held on
        // a different dim, refuse to register.
        for row in &st.rows {
            if row.vector.len() != dim {
                return Err(Error::EmbeddingDimMismatch {
                    model: model.to_string(),
                    stored: row.vector.len(),
                    new: dim,
                });
            }
        }
        let now = unix_now();
        st.dims.insert(
            model.to_string(),
            EmbeddingMetaRow { dim, created_at: now },
        );
        Ok(())
    }

    /// Stored dim for `model`, if any.
    pub async fn embedding_dim_for(&self, model: &str) -> Result<Option<usize>, Error> {
        Ok(self.embeddings.read().await.dim_for(model))
    }

    /// Drop every embedding for `model` plus its meta row.
    pub async fn reset_embeddings(&self, model: &str) -> Result<(), Error> {
        let mut st = self.embeddings.write().await;
        st.reset_model(model);
        Ok(())
    }

    /// Single-row upsert. Marks the index dirty; a subsequent
    /// `vector_top_k` rebuilds the HNSW lazily.
    pub async fn upsert_node_embedding(
        &self,
        node_id: &str,
        model: &str,
        vector: &[f32],
        text_hash: &str,
    ) -> Result<(), Error> {
        let now = unix_now();
        let mut st = self.embeddings.write().await;
        st.upsert(node_id, model, vector, text_hash, now)?;
        Ok(())
    }

    /// `text_hash` of the given row, if any.
    pub async fn embedding_hash_for(
        &self,
        node_id: &str,
        model: &str,
    ) -> Result<Option<String>, Error> {
        let st = self.embeddings.read().await;
        Ok(st
            .by_key
            .get(&(node_id.to_string(), model.to_string()))
            .map(|&i| st.rows[i].text_hash.clone()))
    }

    /// Read every `(node_id, text_hash)` pair for `model`. Replacement
    /// for the old `connection().query("SELECT node_id, text_hash …")`
    /// in `commands/embed.rs` so that binary stops poking storage
    /// internals.
    pub async fn embedding_hashes_for(
        &self,
        model: &str,
    ) -> Result<HashMap<String, String>, Error> {
        let st = self.embeddings.read().await;
        let mut out = HashMap::new();
        for r in &st.rows {
            if r.model == model {
                out.insert(r.node_id.clone(), r.text_hash.clone());
            }
        }
        Ok(out)
    }

    /// Drop the embeddings of every node listed.
    pub async fn forget_embeddings(&self, node_ids: &[String]) -> Result<(), Error> {
        let mut st = self.embeddings.write().await;
        st.forget(node_ids);
        Ok(())
    }

    // ---- LLM output cache -------------------------------------------------

    /// Look up a cached LLM response for `(node_id, prompt_hash)`.
    ///
    /// Returns `Some(response)` only when the entry exists *and* the
    /// stored `file_hash` matches `file_hash` — i.e. the file the LLM
    /// originally saw is byte-identical to the one the caller has now.
    /// A hash mismatch is treated as a miss; the caller should re-run
    /// the LLM and re-cache via [`Self::cache_llm_output`].
    pub async fn llm_output_for(
        &self,
        node_id: &str,
        prompt_hash: &str,
        file_hash: &str,
    ) -> Result<Option<String>, Error> {
        let cache = self.llm_cache.read().await;
        let key = LlmCacheKey {
            node_id: node_id.to_string(),
            prompt_hash: prompt_hash.to_string(),
        };
        match cache.entries.get(&key) {
            Some(entry) if entry.file_hash == file_hash => Ok(Some(entry.response.clone())),
            _ => Ok(None),
        }
    }

    /// Insert or replace the cached response for
    /// `(node_id, prompt_hash)`. Stamps the entry with the provided
    /// `file_hash` and the current unix time.
    pub async fn cache_llm_output(
        &self,
        node_id: &str,
        prompt_hash: &str,
        file_hash: &str,
        response: &str,
    ) -> Result<(), Error> {
        let mut cache = self.llm_cache.write().await;
        let key = LlmCacheKey {
            node_id: node_id.to_string(),
            prompt_hash: prompt_hash.to_string(),
        };
        cache.entries.insert(
            key,
            LlmCacheEntry {
                file_hash: file_hash.to_string(),
                response: response.to_string(),
                created_at: unix_now(),
            },
        );
        Ok(())
    }

    /// Drop every cache entry whose `node_id` is listed in `drop`.
    /// Used by the analyze pipeline when files vanish from the working
    /// tree — keeping stale entries around would just bloat the
    /// archive without ever serving a hit again.
    pub async fn forget_llm_outputs(&self, drop: &[String]) -> Result<(), Error> {
        if drop.is_empty() {
            return Ok(());
        }
        let drop_set: std::collections::HashSet<&str> =
            drop.iter().map(|s| s.as_str()).collect();
        let mut cache = self.llm_cache.write().await;
        cache.entries.retain(|k, _| !drop_set.contains(k.node_id.as_str()));
        Ok(())
    }

    /// Top-K cosine ranker. Will attempt to mmap an `Index::view` from
    /// the cold-open dump on the first query for `model`; falls back
    /// to building / reusing an in-process index if no view is
    /// available or the dump is unusable. The result's `distance` is
    /// usearch's cosine distance (`1 - cos_sim`) — same semantics as
    /// the previous hnsw_rs contract.
    pub async fn vector_top_k(
        &self,
        model: &str,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<VectorHit>, Error> {
        if let Some(dim) = self.embedding_dim_for(model).await? {
            if query.len() != dim {
                return Err(Error::EmbeddingDimMismatch {
                    model: model.to_string(),
                    stored: dim,
                    new: query.len(),
                });
            }
        }
        let mut st = self.embeddings.write().await;

        // 1. If a stale index hangs around for the wrong model, drop it.
        if let IndexState::Built { model: m, .. } | IndexState::View { model: m, .. } =
            &st.index
        {
            if m != model {
                st.index = IndexState::None;
                st.index_dirty = true;
            }
        }

        // 2. If we have a pending view dump and the live state is clean,
        //    try the mmap path first. Cheap on success (no O(N) rebuild).
        if matches!(st.index, IndexState::None) && !st.index_dirty {
            // index_dirty == false means rows match the dump.
            let _ = st.try_install_view_for(model);
        }
        // 3. Otherwise rebuild from scratch.
        if matches!(st.index, IndexState::None) || st.index_dirty {
            st.rebuild_index_for(model)?;
        }

        let index = match &st.index {
            IndexState::Built { index, .. } | IndexState::View { index, .. } => index,
            IndexState::None => return Ok(Vec::new()),
        };
        let matches = index
            .search(query, k.max(1))
            .map_err(|e| Error::Hnsw(format!("usearch search: {e}")))?;
        let mut hits: Vec<VectorHit> = Vec::with_capacity(matches.keys.len());
        for (key, dist) in matches.keys.iter().zip(matches.distances.iter()) {
            let idx = *key as usize;
            if idx >= st.rows.len() {
                continue;
            }
            let row = &st.rows[idx];
            if row.model != model {
                continue;
            }
            hits.push(VectorHit {
                node_id: row.node_id.clone(),
                distance: *dist,
            });
        }
        Ok(hits)
    }

    /// Linear-scan ranker (kept for parity with the old API and to
    /// double-check the ANN). Iterates every row for `model`.
    pub async fn vector_scan_top_k(
        &self,
        model: &str,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<VectorHit>, Error> {
        if let Some(dim) = self.embedding_dim_for(model).await? {
            if query.len() != dim {
                return Err(Error::EmbeddingDimMismatch {
                    model: model.to_string(),
                    stored: dim,
                    new: query.len(),
                });
            }
        }
        let st = self.embeddings.read().await;
        let mut scored: Vec<VectorHit> = st
            .rows
            .iter()
            .filter(|r| r.model == model)
            .map(|r| VectorHit {
                node_id: r.node_id.clone(),
                distance: cosine_distance(query, &r.vector),
            })
            .collect();
        scored.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    /// Number of embedding rows registered for `model`.
    pub async fn embedding_count(&self, model: &str) -> Result<u64, Error> {
        Ok(self.embeddings.read().await.count_for(model))
    }

    /// Bulk-upsert N rows. Equivalent to N `upsert_node_embedding`
    /// calls but only flips the dirty flag once.
    pub async fn upsert_node_embeddings_batch(
        &self,
        model: &str,
        rows: &[EmbeddingBatchRow<'_>],
    ) -> Result<(), Error> {
        if rows.is_empty() {
            return Ok(());
        }
        let now = unix_now();
        let mut st = self.embeddings.write().await;
        for (node_id, text_hash, vector) in rows {
            st.upsert(node_id, model, vector, text_hash, now)?;
        }
        Ok(())
    }

}

// ---- helpers ---------------------------------------------------------------

/// Pick the unique model name held in `EmbeddingsState`, if any. Used
/// by the legacy single-blob cold-open view installer (which can't
/// disambiguate models) and as a sole-model probe at save time. Newer
/// multi-model save paths use [`registered_models`] instead.
fn sole_model(st: &EmbeddingsState) -> Option<String> {
    let mut iter = registered_models(st).into_iter();
    let first = iter.next()?;
    if iter.next().is_none() {
        Some(first)
    } else {
        None
    }
}

/// Every model registered in `EmbeddingsState` — by row or by
/// `dims` meta — sorted to keep the archive contents deterministic.
/// Returns an empty vec for fresh / cleared states.
fn registered_models(st: &EmbeddingsState) -> Vec<String> {
    registered_models_from_snapshot(&st.rows, &st.dims)
}

/// Snapshot-friendly variant of [`registered_models`]. Used by
/// `save_kind` so the dump phase doesn't need to hold any embeddings
/// lock while it walks the rows.
fn registered_models_from_snapshot(
    rows: &[EmbeddingRow],
    dims: &HashMap<String, EmbeddingMetaRow>,
) -> Vec<String> {
    let set: std::collections::BTreeSet<&str> = rows
        .iter()
        .map(|r| r.model.as_str())
        .chain(dims.keys().map(|s| s.as_str()))
        .collect();
    set.into_iter().map(|s| s.to_string()).collect()
}

/// Tarball entry name for a per-model usearch dump.
///
/// Format: `vectors.<model>.usearch`. The model name is taken
/// verbatim — embedding model names in this codebase are already
/// shaped like identifiers (`text-embedding-3-small`, `bge-large-en`,
/// …) so we don't sanitize, and the matching reader is exact-match
/// based.
fn per_model_vectors_entry_name(model: &str) -> String {
    format!("vectors.{model}.usearch")
}

/// Inverse of [`per_model_vectors_entry_name`]. Returns the model
/// name when `name` looks like `vectors.<model>.usearch`, else
/// `None`. The bare `vectors.usearch` entry is *not* parsed here —
/// it's handled separately as the legacy sole-model dump.
fn parse_per_model_vectors_entry(name: &str) -> Option<String> {
    let stripped = name.strip_prefix("vectors.")?.strip_suffix(".usearch")?;
    if stripped.is_empty() {
        // Would round-trip back to "vectors..usearch" — clearly not us.
        return None;
    }
    Some(stripped.to_string())
}

/// Decide whether to keep a cold-open dump in RAM or spill it to a
/// tempfile up front, then build the matching [`PendingView`]. Bytes
/// under [`MAX_LAZY_VIEW_BYTES`] are kept lazy — they only land on
/// disk when the first `vector_top_k(model)` actually fires. Larger
/// dumps are spilled immediately to keep process memory bounded.
///
/// On spill failure we fall back to keeping the bytes in memory and
/// log a warning — the next `try_install_view_for` will retry the
/// spill, and worst case the rebuild path takes over.
fn build_pending_view(bytes: Vec<u8>, model: &str) -> PendingView {
    if bytes.len() <= MAX_LAZY_VIEW_BYTES {
        tracing::debug!(
            %model,
            len = bytes.len(),
            "stashing usearch dump bytes in-memory until first query"
        );
        return PendingView::Bytes(bytes);
    }
    tracing::debug!(
        %model,
        len = bytes.len(),
        cap = MAX_LAZY_VIEW_BYTES,
        "spilling usearch dump to tempfile (over lazy-view cap)"
    );
    match stash_view_blob(&bytes) {
        Ok((tmp, path)) => PendingView::Spilled { _tmp: tmp, path },
        Err(e) => {
            tracing::warn!(
                error = %e,
                %model,
                "failed to spill usearch dump — falling back to lazy bytes"
            );
            PendingView::Bytes(bytes)
        }
    }
}

/// Spill the bytes of one usearch dump into a tempfile and return
/// (`tempfile`, `path`). Caller stashes both — the `EmbeddingsState`
/// keeps the tempfile alive so the `Index::view` mmap stays valid.
fn stash_view_blob(
    bytes: &[u8],
) -> Result<(tempfile::NamedTempFile, PathBuf), Error> {
    let tmp = tempfile::Builder::new()
        .prefix("ua-usearch-view-")
        .suffix(".usearch")
        .tempfile()?;
    {
        use std::io::Write as _;
        let mut f = tmp.reopen()?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let path = tmp.path().to_path_buf();
    Ok((tmp, path))
}

/// Build a usearch index for `model` from a *snapshot* of
/// `EmbeddingRow`s and dump it to a tempfile, then slurp the bytes
/// back. Skips entirely if no rows match `model`.
///
/// Crucially this does *not* touch live `EmbeddingsState`; the caller
/// passes an owned `Vec<EmbeddingRow>` that was cloned out under the
/// read lock and then dropped. That means the dashboard's
/// `vector_top_k` keeps using the live index while the on-disk dump
/// is being built.
///
/// Trade-off: the read snapshot of rows lags the live state by
/// however long the snapshot took. The next `save_kind` picks up the
/// drift; this is the same staleness window we already accepted for
/// fingerprints.
fn build_dump_from_rows(rows: &[EmbeddingRow], model: &str) -> Result<Vec<u8>, Error> {
    let model_rows: Vec<&EmbeddingRow> =
        rows.iter().filter(|r| r.model == model).collect();
    if model_rows.is_empty() {
        return Err(Error::Hnsw(format!(
            "no rows registered for model `{model}` — nothing to dump"
        )));
    }
    let dim = model_rows[0].vector.len();
    let options = build_index_options(dim);
    let index = Index::new(&options).map_err(|e| Error::Hnsw(format!("usearch new: {e}")))?;
    index
        .reserve(model_rows.len().max(16))
        .map_err(|e| Error::Hnsw(format!("usearch reserve: {e}")))?;
    // The u64 keys must match the live state's row positions — that's
    // the contract `vector_top_k` relies on after a cold-open install.
    // Since we cloned the rows in original order, `enumerate()` yields
    // the same positions. We index *into the snapshot*, not into the
    // filter, which would shift the keys.
    for (i, row) in rows.iter().enumerate() {
        if row.model != model {
            continue;
        }
        index
            .add(i as u64, &row.vector)
            .map_err(|e| Error::Hnsw(format!("usearch add: {e}")))?;
    }
    let tmp = tempfile::Builder::new()
        .prefix("ua-usearch-dump-")
        .suffix(".usearch")
        .tempfile()?;
    let path = tmp.path().to_path_buf();
    index
        .save(path.to_string_lossy().as_ref())
        .map_err(|e| Error::Hnsw(format!("usearch save: {e}")))?;
    let bytes = std::fs::read(&path)?;
    Ok(bytes)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 1.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = (na.sqrt() * nb.sqrt()).max(1e-12);
    1.0 - (dot / denom)
}

/// Decode an `embeddings.bin` body into the empty
/// [`EmbeddingsState`]. Header is bincode; vectors are raw f32 LE
/// bytes appended in row order.
///
/// Fails fast with [`Error::Schema`] if the header version doesn't
/// match [`EMBEDDINGS_HEADER_VERSION`] — silently dropping a future
/// payload would lose data. To migrate, replace the equality check
/// here with a `match header.version { 1 => …, _ => Error::Schema }`
/// branch and translate the older payload into the current shape.
fn decode_embeddings(bytes: &[u8], st: &mut EmbeddingsState) -> Result<(), Error> {
    if bytes.is_empty() {
        return Ok(());
    }
    let mut cursor = std::io::Cursor::new(bytes);
    let header: EmbeddingsHeader =
        bincode::deserialize_from(&mut cursor).map_err(|e| Error::Bincode(e.to_string()))?;
    if header.version != EMBEDDINGS_HEADER_VERSION {
        return Err(Error::Schema(format!(
            "embeddings header version mismatch: got {got}, expected {expected}",
            got = header.version,
            expected = EMBEDDINGS_HEADER_VERSION
        )));
    }
    let header_end = cursor.position() as usize;
    let body = &bytes[header_end..];
    let mut offset = 0usize;
    for row in &header.rows {
        let nbytes = row.dim * std::mem::size_of::<f32>();
        if offset + nbytes > body.len() {
            return Err(Error::Storage(format!(
                "embeddings body truncated: row {} expected {nbytes} bytes",
                row.node_id
            )));
        }
        let slice = &body[offset..offset + nbytes];
        let mut vec: Vec<f32> = vec![0.0; row.dim];
        bytemuck::cast_slice_mut(&mut vec[..]).copy_from_slice(slice);
        offset += nbytes;
        let key = (row.node_id.clone(), row.model.clone());
        let idx = st.rows.len();
        st.by_key.insert(key, idx);
        st.rows.push(EmbeddingRow {
            node_id: row.node_id.clone(),
            model: row.model.clone(),
            vector: vec,
            text_hash: row.text_hash.clone(),
            updated_at: row.updated_at,
        });
    }
    Ok(())
}

/// Encode a row snapshot into the wire format consumed by
/// [`decode_embeddings`]. Used by `save_kind`, which deliberately
/// works on a `Vec<EmbeddingRow>` clone instead of the live
/// `EmbeddingsState` so the embeddings lock isn't held across the
/// dump.
fn encode_embeddings_from_snapshot(rows: &[EmbeddingRow]) -> Result<Vec<u8>, Error> {
    let header = EmbeddingsHeader {
        version: EMBEDDINGS_HEADER_VERSION,
        rows: rows
            .iter()
            .map(|r| EmbeddingsHeaderRow {
                node_id: r.node_id.clone(),
                model: r.model.clone(),
                dim: r.vector.len(),
                text_hash: r.text_hash.clone(),
                updated_at: r.updated_at,
            })
            .collect(),
    };
    let mut out = bincode::serialize(&header).map_err(|e| Error::Bincode(e.to_string()))?;
    for r in rows {
        let raw: &[u8] = bytemuck::cast_slice(&r.vector);
        out.extend_from_slice(raw);
    }
    Ok(out)
}

fn decode_msgpack_db(bytes: &[u8]) -> Result<Database<MemoryDatastore>, Error> {
    // The crate only exposes a path-based reader, so we hand it a
    // tempfile holding `bytes`. The file lives until `read_msgpack_db`
    // returns; the `Database` no longer points at it.
    let mut tmp = tempfile::Builder::new()
        .prefix("ua-graph-")
        .suffix(".msgpack")
        .tempfile()?;
    {
        use std::io::Write as _;
        tmp.as_file_mut().write_all(bytes)?;
        tmp.as_file_mut().sync_all()?;
    }
    let path = tmp.path().to_path_buf();
    let db = MemoryDatastore::read_msgpack_db(&path)
        .map_err(|e| Error::Graph(format!("read_msgpack_db: {e}")))?;
    Ok(db)
}

fn count_query(db: &Database<MemoryDatastore>, q: impl Into<IndraQuery>) -> Result<u64, Error> {
    let q = q.into();
    let res = db
        .get(q)
        .map_err(|e| Error::Graph(format!("count: {e}")))?
        .pop();
    match res {
        Some(QueryOutputValue::Vertices(v)) => Ok(v.len() as u64),
        Some(QueryOutputValue::Edges(e)) => Ok(e.len() as u64),
        _ => Ok(0),
    }
}

// ---- enum mappers ---------------------------------------------------------

fn node_type_str(t: NodeType) -> &'static str {
    t.as_str()
}

fn parse_node_type(s: &str) -> Option<NodeType> {
    NodeType::ALL.iter().copied().find(|n| n.as_str() == s)
}

fn edge_type_str(t: EdgeType) -> &'static str {
    match t {
        EdgeType::Imports => "imports",
        EdgeType::Exports => "exports",
        EdgeType::Contains => "contains",
        EdgeType::Inherits => "inherits",
        EdgeType::Implements => "implements",
        EdgeType::Calls => "calls",
        EdgeType::Subscribes => "subscribes",
        EdgeType::Publishes => "publishes",
        EdgeType::Middleware => "middleware",
        EdgeType::ReadsFrom => "reads_from",
        EdgeType::WritesTo => "writes_to",
        EdgeType::Transforms => "transforms",
        EdgeType::Validates => "validates",
        EdgeType::DependsOn => "depends_on",
        EdgeType::TestedBy => "tested_by",
        EdgeType::Configures => "configures",
        EdgeType::Related => "related",
        EdgeType::SimilarTo => "similar_to",
        EdgeType::Deploys => "deploys",
        EdgeType::Serves => "serves",
        EdgeType::Provisions => "provisions",
        EdgeType::Triggers => "triggers",
        EdgeType::Migrates => "migrates",
        EdgeType::Documents => "documents",
        EdgeType::Routes => "routes",
        EdgeType::DefinesSchema => "defines_schema",
        EdgeType::ContainsFlow => "contains_flow",
        EdgeType::FlowStep => "flow_step",
        EdgeType::CrossDomain => "cross_domain",
        EdgeType::Cites => "cites",
        EdgeType::Contradicts => "contradicts",
        EdgeType::BuildsOn => "builds_on",
        EdgeType::Exemplifies => "exemplifies",
        EdgeType::CategorizedUnder => "categorized_under",
        EdgeType::AuthoredBy => "authored_by",
    }
}

fn parse_edge_type(s: &str) -> Option<EdgeType> {
    EdgeType::ALL.iter().copied().find(|t| edge_type_str(*t) == s)
}

fn complexity_str(c: ua_core::Complexity) -> &'static str {
    match c {
        ua_core::Complexity::Simple => "simple",
        ua_core::Complexity::Moderate => "moderate",
        ua_core::Complexity::Complex => "complex",
    }
}

fn parse_complexity(s: &str) -> ua_core::Complexity {
    match s {
        "simple" => ua_core::Complexity::Simple,
        "complex" => ua_core::Complexity::Complex,
        _ => ua_core::Complexity::Moderate,
    }
}

fn edge_direction_str(d: EdgeDirection) -> &'static str {
    match d {
        EdgeDirection::Forward => "forward",
        EdgeDirection::Backward => "backward",
        EdgeDirection::Bidirectional => "bidirectional",
    }
}

fn parse_direction(s: &str) -> EdgeDirection {
    match s {
        "backward" => EdgeDirection::Backward,
        "bidirectional" => EdgeDirection::Bidirectional,
        _ => EdgeDirection::Forward,
    }
}

fn graph_kind_str(k: GraphKind) -> &'static str {
    match k {
        GraphKind::Codebase => "codebase",
        GraphKind::Knowledge => "knowledge",
        GraphKind::Domain => "domain",
    }
}

fn parse_graph_kind(s: &str) -> Option<GraphKind> {
    match s {
        "codebase" => Some(GraphKind::Codebase),
        "knowledge" => Some(GraphKind::Knowledge),
        "domain" => Some(GraphKind::Domain),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Internal storage tests that need to peek at private state.
    //! Public-API end-to-end tests live in `tests/vector_search.rs`.

    use super::*;
    use crate::layout::ProjectLayout;

    #[test]
    fn parse_per_model_vectors_entry_round_trip() {
        let name = per_model_vectors_entry_name("text-embedding-3-small");
        assert_eq!(name, "vectors.text-embedding-3-small.usearch");
        assert_eq!(
            parse_per_model_vectors_entry(&name).as_deref(),
            Some("text-embedding-3-small")
        );
    }

    #[test]
    fn parse_per_model_vectors_entry_rejects_legacy_and_garbage() {
        // Legacy single-blob name must not match the per-model parser
        // — `from_archive` handles it on the dedicated branch.
        assert_eq!(parse_per_model_vectors_entry("vectors.usearch"), None);
        assert_eq!(parse_per_model_vectors_entry("graph.msgpack"), None);
        assert_eq!(parse_per_model_vectors_entry("vectors..usearch"), None);
        assert_eq!(parse_per_model_vectors_entry("vectors.foo"), None);
        assert_eq!(parse_per_model_vectors_entry("foo.bar.usearch"), None);
    }

    /// Multi-model save → reopen → vector_top_k for each model. Both
    /// models must resolve their nearest hit on first query, which —
    /// for a clean reopen — exercises the per-model cold-open view
    /// path inside `try_install_view_for`.
    #[tokio::test(flavor = "current_thread")]
    async fn multi_model_archive_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());

        // Build a tiny graph with 4 file nodes (so node ids resolve
        // through `id_map`); keep it minimal — we only care about
        // embedding round-trip.
        use ua_core::{
            Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
        };
        let mk_node = |id: &str, name: &str| GraphNode {
            id: id.into(),
            node_type: NodeType::File,
            name: name.into(),
            file_path: Some(format!("{name}.rs")),
            line_range: None,
            summary: String::new(),
            tags: vec![],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        };
        let graph = KnowledgeGraph {
            version: "0.1.0".into(),
            kind: Some(GraphKind::Codebase),
            project: ProjectMeta {
                name: "mm".into(),
                languages: vec![],
                frameworks: vec![],
                description: String::new(),
                analyzed_at: String::new(),
                git_commit_hash: String::new(),
            },
            nodes: vec![
                mk_node("n:a", "alpha"),
                mk_node("n:b", "bravo"),
                mk_node("n:c", "charlie"),
                mk_node("n:d", "delta"),
            ],
            edges: vec![],
            layers: vec![],
            tour: vec![],
        };

        // Two distinct models with distinct dims so the per-model
        // dump bookkeeping really has to stay model-aware. Model A is
        // 4-dim, model B is 3-dim — no aliasing possible.
        {
            let s = Storage::open(&layout).await.unwrap();
            s.save_graph(&graph).await.unwrap();
            s.ensure_embeddings_table("model-a", 4).await.unwrap();
            s.ensure_embeddings_table("model-b", 3).await.unwrap();

            // model-a: each node on a distinct axis (4-dim).
            for (id, vec) in [
                ("n:a", [1.0_f32, 0.0, 0.0, 0.0]),
                ("n:b", [0.0, 1.0, 0.0, 0.0]),
                ("n:c", [0.0, 0.0, 1.0, 0.0]),
                ("n:d", [0.0, 0.0, 0.0, 1.0]),
            ] {
                s.upsert_node_embedding(id, "model-a", &vec, "ha")
                    .await
                    .unwrap();
            }
            // model-b: a totally different geometry (3-dim).
            for (id, vec) in [
                ("n:a", [1.0_f32, 0.0, 0.0]),
                ("n:b", [0.0, 1.0, 0.0]),
                ("n:c", [0.0, 0.0, 1.0]),
                ("n:d", [0.5, 0.5, 0.0]),
            ] {
                s.upsert_node_embedding(id, "model-b", &vec, "hb")
                    .await
                    .unwrap();
            }
            s.save(&layout).await.unwrap();
        }

        // The archive should now carry one per-model dump per model
        // (multi-model branch of `save_kind`). Inspect the tarball
        // directly so we don't rely on private state.
        let archive_path = layout.graph_archive();
        let bytes = std::fs::read(&archive_path).unwrap();
        let dec = zstd::stream::read::Decoder::new(&bytes[..]).unwrap();
        let mut tar = tar::Archive::new(dec);
        let mut found_a = false;
        let mut found_b = false;
        let mut legacy_present = false;
        for e in tar.entries().unwrap() {
            let e = e.unwrap();
            let p = e.path().unwrap().to_string_lossy().into_owned();
            match p.as_str() {
                "vectors.model-a.usearch" => found_a = true,
                "vectors.model-b.usearch" => found_b = true,
                "vectors.usearch" => legacy_present = true,
                _ => {}
            }
        }
        assert!(found_a, "archive must carry per-model dump for model-a");
        assert!(found_b, "archive must carry per-model dump for model-b");
        assert!(
            !legacy_present,
            "multi-model archives must not write the legacy single blob"
        );

        // Reopen and query each model — both should produce results
        // and pick the right top hit. With a clean reopen + no
        // mutations, the cold-open per-model view is consumed on
        // first query.
        let s = Storage::open(&layout).await.unwrap();

        let q_a = vec![0.05_f32, 0.99, 0.05, 0.05]; // closest to bravo on axis 1
        let hits_a = s.vector_top_k("model-a", &q_a, 4).await.unwrap();
        assert_eq!(hits_a.len(), 4, "model-a hits: {hits_a:?}");
        assert_eq!(hits_a[0].node_id, "n:b", "model-a top: {hits_a:?}");

        let q_b = vec![0.05_f32, 0.05, 0.99]; // closest to charlie on axis 2
        let hits_b = s.vector_top_k("model-b", &q_b, 4).await.unwrap();
        assert_eq!(hits_b.len(), 4, "model-b hits: {hits_b:?}");
        assert_eq!(hits_b[0].node_id, "n:c", "model-b top: {hits_b:?}");
    }

    /// Sole-model save still emits the legacy `vectors.usearch` entry
    /// (so older readers and the existing `cold_open_uses_view`
    /// integration test stay green) — and reopen still fast-paths via
    /// the cold-open view.
    #[tokio::test(flavor = "current_thread")]
    async fn sole_model_archive_keeps_legacy_blob() {
        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());

        use ua_core::{
            Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
        };
        let mk_node = |id: &str| GraphNode {
            id: id.into(),
            node_type: NodeType::File,
            name: id.into(),
            file_path: Some(format!("{id}.rs")),
            line_range: None,
            summary: String::new(),
            tags: vec![],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        };
        let graph = KnowledgeGraph {
            version: "0.1.0".into(),
            kind: Some(GraphKind::Codebase),
            project: ProjectMeta {
                name: "s".into(),
                languages: vec![],
                frameworks: vec![],
                description: String::new(),
                analyzed_at: String::new(),
                git_commit_hash: String::new(),
            },
            nodes: vec![mk_node("n:x"), mk_node("n:y")],
            edges: vec![],
            layers: vec![],
            tour: vec![],
        };

        {
            let s = Storage::open(&layout).await.unwrap();
            s.save_graph(&graph).await.unwrap();
            s.ensure_embeddings_table("only", 2).await.unwrap();
            s.upsert_node_embedding("n:x", "only", &[1.0, 0.0], "h")
                .await
                .unwrap();
            s.upsert_node_embedding("n:y", "only", &[0.0, 1.0], "h")
                .await
                .unwrap();
            s.save(&layout).await.unwrap();
        }

        let bytes = std::fs::read(layout.graph_archive()).unwrap();
        let dec = zstd::stream::read::Decoder::new(&bytes[..]).unwrap();
        let mut tar = tar::Archive::new(dec);
        let mut legacy = false;
        let mut per_model = false;
        for e in tar.entries().unwrap() {
            let e = e.unwrap();
            let p = e.path().unwrap().to_string_lossy().into_owned();
            if p == "vectors.usearch" {
                legacy = true;
            } else if p == "vectors.only.usearch" {
                per_model = true;
            }
        }
        assert!(legacy, "sole-model archive must keep writing legacy blob");
        assert!(
            !per_model,
            "sole-model archive must not also write a per-model blob"
        );

        let s = Storage::open(&layout).await.unwrap();
        let hits = s.vector_top_k("only", &[0.0, 1.0], 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].node_id, "n:y");
    }

    /// Cache a response, save, reopen, and confirm we get a hit on the
    /// matching `(node_id, prompt_hash, file_hash)` triple. This
    /// exercises serialise + bincode round-trip + tarball entry + the
    /// `from_archive` decode path.
    #[tokio::test(flavor = "current_thread")]
    async fn llm_output_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());

        {
            let s = Storage::open(&layout).await.unwrap();
            s.cache_llm_output(
                "file:src/foo.rs",
                "prompt-hash-1",
                "file-hash-abc",
                "summary text",
            )
            .await
            .unwrap();
            s.save(&layout).await.unwrap();
        }

        let s = Storage::open(&layout).await.unwrap();
        let hit = s
            .llm_output_for("file:src/foo.rs", "prompt-hash-1", "file-hash-abc")
            .await
            .unwrap();
        assert_eq!(hit.as_deref(), Some("summary text"));
    }

    /// A cache entry stamped with one `file_hash` must NOT serve a
    /// query carrying a different hash — that's exactly the
    /// invalidation path that protects us from stale summaries on edited
    /// files.
    #[tokio::test(flavor = "current_thread")]
    async fn llm_output_cache_file_hash_mismatch_misses() {
        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());

        let s = Storage::open(&layout).await.unwrap();
        s.cache_llm_output(
            "file:src/foo.rs",
            "prompt-hash-1",
            "old-hash",
            "stale summary",
        )
        .await
        .unwrap();

        // Same key, different file content → miss.
        let miss = s
            .llm_output_for("file:src/foo.rs", "prompt-hash-1", "new-hash")
            .await
            .unwrap();
        assert!(miss.is_none(), "stale entry must not be served");

        // Same key + matching hash → hit.
        let hit = s
            .llm_output_for("file:src/foo.rs", "prompt-hash-1", "old-hash")
            .await
            .unwrap();
        assert_eq!(hit.as_deref(), Some("stale summary"));

        // forget_llm_outputs drops the entry entirely.
        s.forget_llm_outputs(&["file:src/foo.rs".to_string()])
            .await
            .unwrap();
        let gone = s
            .llm_output_for("file:src/foo.rs", "prompt-hash-1", "old-hash")
            .await
            .unwrap();
        assert!(gone.is_none(), "forget_llm_outputs must drop the entry");
    }

    /// Spawn `save_kind` and `save_graph` against the same `Storage`
    /// in tight loops; both must complete within a generous timeout.
    /// This is the canary for issue #1 — the previous lock order had
    /// `save_kind` holding `embeddings → meta → graph` while
    /// `save_graph` held `graph → meta`, which interleaves into a
    /// dependency cycle. The fix is the canonical lock order
    /// documented on [`Storage`].
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_save_kind_and_save_graph_does_not_deadlock() {
        use std::sync::Arc;
        use std::time::Duration;
        use ua_core::{
            Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
        };

        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());
        let storage = Arc::new(Storage::open(&layout).await.unwrap());

        let mk_node = |id: &str| GraphNode {
            id: id.into(),
            node_type: NodeType::File,
            name: id.into(),
            file_path: Some(format!("{id}.rs")),
            line_range: None,
            summary: String::new(),
            tags: vec![],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        };
        let graph = KnowledgeGraph {
            version: "0.1.0".into(),
            kind: Some(GraphKind::Codebase),
            project: ProjectMeta {
                name: "deadlock".into(),
                languages: vec![],
                frameworks: vec![],
                description: String::new(),
                analyzed_at: String::new(),
                git_commit_hash: String::new(),
            },
            nodes: vec![mk_node("n:a"), mk_node("n:b"), mk_node("n:c")],
            edges: vec![],
            layers: vec![],
            tour: vec![],
        };

        // Seed a couple of embeddings so save_kind has real dump
        // work to do — exercises the same critical sections that
        // were tangled before the fix.
        storage
            .ensure_embeddings_table("m", 3)
            .await
            .unwrap();
        storage
            .upsert_node_embedding("n:a", "m", &[1.0, 0.0, 0.0], "h")
            .await
            .unwrap();
        storage
            .upsert_node_embedding("n:b", "m", &[0.0, 1.0, 0.0], "h")
            .await
            .unwrap();

        let s_save = storage.clone();
        let layout_save = layout.clone();
        let save_kind_task = tokio::spawn(async move {
            for _ in 0..10 {
                s_save.save_kind(&layout_save, "codebase").await.unwrap();
            }
        });

        let s_graph = storage.clone();
        let g = graph.clone();
        let save_graph_task = tokio::spawn(async move {
            for _ in 0..10 {
                s_graph.save_graph(&g).await.unwrap();
            }
        });

        // 5s is generous — both loops together are < 500ms on a cold
        // CI runner. If we time out, something is genuinely
        // deadlocked rather than just slow.
        let res = tokio::time::timeout(
            Duration::from_secs(5),
            async move {
                save_kind_task.await.unwrap();
                save_graph_task.await.unwrap();
            },
        )
        .await;
        assert!(
            res.is_ok(),
            "save_kind / save_graph deadlocked — fix the lock order"
        );
    }

    /// Synthesise an embeddings.bin with a header version we don't
    /// know about; the decoder must reject it with `Error::Schema`
    /// rather than silently accept (or panic on) a future payload.
    #[test]
    fn embeddings_header_version_mismatch_returns_schema_error() {
        // Hand-roll a payload: future version 999, zero rows. No body
        // bytes needed since rows is empty; the decoder fails on the
        // header check before touching the body.
        let header = EmbeddingsHeader {
            version: 999,
            rows: Vec::new(),
        };
        let bytes = bincode::serialize(&header).unwrap();

        let mut st = EmbeddingsState::default();
        let err = decode_embeddings(&bytes, &mut st)
            .expect_err("future header version must error");
        match err {
            Error::Schema(msg) => {
                assert!(
                    msg.contains("999") && msg.contains("expected 1"),
                    "schema error must name both versions: got {msg:?}"
                );
            }
            other => panic!("expected Error::Schema, got {other:?}"),
        }
        // Live state must be untouched on failure.
        assert!(st.rows.is_empty());
    }

    /// `save_kind` must not block concurrent `vector_top_k`. With
    /// the snapshot-based dump path, the embeddings write lock is
    /// never held across the dump phase — read queries should
    /// complete promptly even while a save is in progress.
    ///
    /// We don't pin a tight timing — CI can be variable — but we
    /// do require that 200 queries finish well inside the 5s
    /// timeout regardless of what `save_kind` is doing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn save_kind_does_not_block_concurrent_vector_top_k() {
        use std::sync::Arc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());
        let storage = Arc::new(Storage::open(&layout).await.unwrap());

        // Seed enough rows that the save's index build / encode
        // phase has real work; 256 4-dim rows is plenty.
        storage.ensure_embeddings_table("m", 4).await.unwrap();
        for i in 0..256u32 {
            let v = [
                (i & 1) as f32,
                ((i >> 1) & 1) as f32,
                ((i >> 2) & 1) as f32,
                ((i >> 3) & 1) as f32,
            ];
            storage
                .upsert_node_embedding(&format!("n:{i}"), "m", &v, "h")
                .await
                .unwrap();
        }

        let s_save = storage.clone();
        let layout_save = layout.clone();
        let saver = tokio::spawn(async move {
            for _ in 0..5 {
                s_save.save_kind(&layout_save, "codebase").await.unwrap();
            }
        });

        let s_query = storage.clone();
        let querier = tokio::spawn(async move {
            for _ in 0..200 {
                let _ = s_query
                    .vector_top_k("m", &[1.0, 0.0, 0.0, 0.0], 5)
                    .await
                    .unwrap();
            }
        });

        let res = tokio::time::timeout(Duration::from_secs(5), async move {
            querier.await.unwrap();
            saver.await.unwrap();
        })
        .await;
        assert!(
            res.is_ok(),
            "vector_top_k blocked by save_kind — snapshot path regressed"
        );
    }

    /// Small dump bytes (well under [`MAX_LAZY_VIEW_BYTES`]) must be
    /// kept in `PendingView::Bytes` until the first `vector_top_k`.
    /// We exercise `build_pending_view` directly so the test
    /// doesn't depend on the archive write/read round-trip.
    #[test]
    fn lazy_view_bytes_under_threshold_kept_in_memory() {
        let small = vec![0u8; 1024];
        let pv = build_pending_view(small.clone(), "m");
        match pv {
            PendingView::Bytes(b) => assert_eq!(b, small),
            PendingView::Spilled { .. } => {
                panic!("small dump must stay lazy in memory")
            }
        }
    }

    /// Dumps over [`MAX_LAZY_VIEW_BYTES`] must spill to a tempfile
    /// at open time so we don't pin a multi-hundred-MB blob in
    /// process memory until the first query.
    #[test]
    fn lazy_view_bytes_above_threshold_spilled_immediately() {
        // Synthesise a payload one byte over the cap. Allocating
        // ~64MB inside a unit test is fine — it's freed at the end
        // of the scope.
        let big = vec![0u8; MAX_LAZY_VIEW_BYTES + 1];
        let pv = build_pending_view(big, "m");
        match pv {
            PendingView::Spilled { path, .. } => {
                assert!(
                    path.exists(),
                    "spilled tempfile must exist on disk"
                );
            }
            PendingView::Bytes(_) => {
                panic!("over-cap dump must spill immediately")
            }
        }
    }

    /// A bincode payload stamped with a future
    /// `LlmOutputCache::version` must be rejected with
    /// `Error::Schema` on archive open. Silently dropping a future
    /// payload would lose work the user expects to survive.
    #[tokio::test(flavor = "current_thread")]
    async fn llm_output_cache_rejects_future_version() {
        let dir = tempfile::tempdir().unwrap();
        let layout = ProjectLayout::under(dir.path());

        // First, produce a valid archive via the public API so we
        // have all the other entries (meta, graph, …). Then we
        // unpack, replace `llm_outputs.bincode` with a future-version
        // payload, repack, and reopen.
        {
            let s = Storage::open(&layout).await.unwrap();
            s.save(&layout).await.unwrap();
        }
        let archive_path = layout.graph_archive();
        let bytes = std::fs::read(&archive_path).unwrap();
        let dec = zstd::stream::read::Decoder::new(&bytes[..]).unwrap();
        let mut tar = tar::Archive::new(dec);
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for e in tar.entries().unwrap() {
            let mut e = e.unwrap();
            let name = e.path().unwrap().to_string_lossy().into_owned();
            use std::io::Read as _;
            let mut buf = Vec::new();
            e.read_to_end(&mut buf).unwrap();
            entries.push((name, buf));
        }

        // Build a future-version cache payload manually. We skip
        // serde_json::Value-style derive because `LlmOutputCache`
        // owns the version field — just stamp it directly.
        let future_cache = LlmOutputCache {
            version: 999,
            entries: HashMap::new(),
        };
        let future_bytes = bincode::serialize(&future_cache).unwrap();
        let mut found = false;
        for entry in entries.iter_mut() {
            if entry.0 == entry::LLM_OUTPUTS_BINCODE {
                entry.1 = future_bytes.clone();
                found = true;
            }
        }
        assert!(found, "save() must have written llm_outputs.bincode");

        archive::write_archive(&archive_path, entries).unwrap();

        // Reopening must surface Error::Schema. We can't use
        // `Storage::open` because it maps a missing `meta` to a
        // fresh empty store — we need the archive to actually be
        // read, which it is for our valid+patched archive.
        let res = Storage::open(&layout).await;
        match res {
            Ok(_) => panic!("future llm_outputs version must error"),
            Err(Error::Schema(msg)) => {
                assert!(
                    msg.contains("999") && msg.contains("expected 1"),
                    "schema error must name both versions: got {msg:?}"
                );
            }
            Err(other) => panic!("expected Error::Schema, got {other:?}"),
        }
    }

    /// Legacy archives written before the `structural_hash` field
    /// existed encoded `FingerprintRow` with three fields. The new
    /// shape adds an optional fourth — `#[serde(default)]` must keep
    /// the old payload deserialising cleanly so nobody has to
    /// re-fingerprint a project just to upgrade.
    #[test]
    fn fingerprint_row_legacy_archive_round_trips_without_structural_hash() {
        // Synthesize a "before" payload with only the three legacy
        // keys. We use a JSON probe rather than bincode to keep the
        // test independent of the on-disk encoding details — the
        // `#[serde(default)]` semantics apply to any serde frontend.
        let legacy = serde_json::json!({
            "path": "src/lib.rs",
            "hash": "deadbeef",
            "modified_at": 1_700_000_000_i64,
        });
        let row: FingerprintRow = serde_json::from_value(legacy)
            .expect("legacy row without structural_hash must deserialize");
        assert_eq!(row.path, "src/lib.rs");
        assert_eq!(row.hash, "deadbeef");
        assert_eq!(row.modified_at, Some(1_700_000_000));
        assert_eq!(
            row.structural_hash, None,
            "missing field must default to None, not error"
        );

        // Round-trip through the public Fingerprint conversion: the
        // resulting Fingerprint must also carry None and a re-encode
        // must omit the field (skip_serializing_if).
        let fp: Fingerprint = row.into();
        assert_eq!(fp.structural_hash, None);
        let row_again: FingerprintRow = (&fp).into();
        let json = serde_json::to_value(&row_again).unwrap();
        assert!(
            json.get("structural_hash").is_none(),
            "skip_serializing_if must drop None: got {json}"
        );
    }

    /// New archives write the structural hash; reading them back must
    /// preserve the value end-to-end.
    #[test]
    fn fingerprint_row_with_structural_hash_round_trips() {
        let fp = Fingerprint {
            path: "src/lib.rs".into(),
            hash: "deadbeef".into(),
            modified_at: Some(42),
            structural_hash: Some("cafef00d".into()),
        };
        let row: FingerprintRow = (&fp).into();
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(
            json.get("structural_hash").and_then(|v| v.as_str()),
            Some("cafef00d")
        );
        let parsed: FingerprintRow = serde_json::from_value(json).unwrap();
        let back: Fingerprint = parsed.into();
        assert_eq!(back, fp);
    }
}
