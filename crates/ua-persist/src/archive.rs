//! tar.zst archive packer / unpacker.
//!
//! The persisted form of one Storage instance lives in a single
//! `<db_name>.tar.zst` file, written via the standard
//! tmp + fsync + rename + parent-dir fsync dance so a SIGKILL anywhere
//! in the sequence leaves either the previous archive intact or — at
//! worst — a stray `<dst>.tmp` that callers ignore.
//!
//! # Layout inside the tarball
//!
//! ```text
//! meta.json            # small JSON: schema version, project_root, fingerprints,
//!                      #             layers, tour, embedding meta, counts.
//! id_map.bincode       # HashMap<String, Uuid> — business key -> vertex id.
//! graph.msgpack        # IndraDB MemoryDatastore msgpack snapshot.
//! embeddings.bin       # raw f32 vectors with a small bincode header
//!                      # (model name, dim, count, per-row metadata).
//!                      # SOURCE OF TRUTH — `vectors.usearch` is rebuildable.
//! vectors.usearch      # OPTIONAL: usearch::Index::save output (single file).
//!                      # When present we mmap it via `Index::view` for
//!                      # cold-open zero-copy queries; rebuild lazily on
//!                      # the first mutation.
//! ```
//!
//! Schema version 3 dropped the legacy `vectors.hnsw.{graph,data}`
//! pair in favour of the single `vectors.usearch` blob. Schema 2 and
//! lower are read-tolerated (legacy entries are ignored, the index is
//! rebuilt lazily from `embeddings.bin`).

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ua_core::Error;

/// Bumped whenever the on-disk layout changes in a way old readers
/// wouldn't tolerate. See [`ArchiveMeta::schema_version`].
///
/// History:
///  * 1: original layout (sqlite-backed).
///  * 2: IndraDB + hnsw_rs (twin `vectors.hnsw.{graph,data}` blobs).
///  * 3: IndraDB + usearch (single `vectors.usearch` blob, mmap-able).
pub const UNDERSTANDABLE_SCHEMA_VERSION: u32 = 3;

/// Compression level for the outer zstd stream.
pub const ZSTD_LEVEL: i32 = 12;

/// Fixed entry names inside the tarball.
pub mod entry {
    pub const META_JSON: &str = "meta.json";
    pub const ID_MAP_BINCODE: &str = "id_map.bincode";
    pub const GRAPH_MSGPACK: &str = "graph.msgpack";
    pub const EMBEDDINGS_BIN: &str = "embeddings.bin";
    /// Single-file usearch dump (schema v3+).
    pub const VECTORS_USEARCH: &str = "vectors.usearch";
    /// LLM output cache — bincode-encoded [`crate::storage::LlmOutputCache`].
    /// Maps `(node_id, prompt_hash)` to the response text emitted by
    /// the LLM the last time that file/prompt pair was seen, plus the
    /// `file_hash` that was current at the time. Re-running
    /// `analyze --with-llm` on an unchanged file (and an unchanged
    /// prompt) skips the LLM call entirely.
    ///
    /// Older readers (schema v3 without this entry) ignore the file —
    /// the entry is captured by the catch-all "unknown archive entry"
    /// branch, which warns but keeps loading. New writers always emit
    /// it so future opens benefit from the cache.
    pub const LLM_OUTPUTS_BINCODE: &str = "llm_outputs.bincode";
    /// Legacy hnsw_rs entries (schema v2). Read-only — we tolerate
    /// them on open and ignore the bytes, then write the new format
    /// on the next save.
    pub const LEGACY_HNSW_GRAPH: &str = "vectors.hnsw.graph";
    pub const LEGACY_HNSW_DATA: &str = "vectors.hnsw.data";
}

/// One file pulled out of a `tar.zst` archive — `name` is the entry
/// path inside the tar, `bytes` is the raw payload.
pub struct ArchiveEntry {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Decompress + untar `path` into memory. Returns an empty vec when
/// `path` does not exist — callers treat this as "fresh archive".
pub fn read_archive(path: &Path) -> Result<Vec<ArchiveEntry>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = File::open(path)?;
    let dec = zstd::stream::read::Decoder::new(f).map_err(|e| Error::Zstd(e.to_string()))?;
    let mut tar = tar::Archive::new(dec);

    let mut out = Vec::new();
    for entry in tar.entries().map_err(|e| Error::Tar(e.to_string()))? {
        let mut entry = entry.map_err(|e| Error::Tar(e.to_string()))?;
        let name = entry
            .path()
            .map_err(|e| Error::Tar(e.to_string()))?
            .to_string_lossy()
            .into_owned();
        let mut bytes = Vec::with_capacity(entry.header().size().unwrap_or(0) as usize);
        entry.read_to_end(&mut bytes)?;
        out.push(ArchiveEntry { name, bytes });
    }
    Ok(out)
}

/// Compose + atomically write a `tar.zst` archive at `path` from a
/// list of `(name, bytes)` entries.
pub fn write_archive(path: &Path, entries: Vec<(String, Vec<u8>)>) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "write_archive: destination has no parent directory",
        ))
    })?;
    if !parent.exists() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    {
        let f = File::create(&tmp)?;
        let enc = zstd::stream::write::Encoder::new(f, ZSTD_LEVEL)
            .map_err(|e| Error::Zstd(e.to_string()))?
            .auto_finish();
        let mut tar = tar::Builder::new(enc);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            tar.append_data(&mut header, &name, bytes.as_slice())
                .map_err(|e| Error::Tar(e.to_string()))?;
        }
        let inner = tar.into_inner().map_err(|e| Error::Tar(e.to_string()))?;
        // `auto_finish()` flushes the zstd footer when the encoder
        // drops; explicit drop here pins the order.
        drop(inner);
    }
    // sync + rename + parent fsync. Open the tmp briefly to fsync it.
    {
        let f = File::open(&tmp)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    sync_parent_dir(parent);
    Ok(())
}

fn tmp_path(dst: &Path) -> PathBuf {
    let mut s = dst.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) {
    if let Ok(f) = File::open(parent) {
        let _ = f.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) {
    // The rename itself is the durability boundary on Windows.
}

/// Crash-safe write of a stand-alone byte blob. Used by tests that
/// poke around inside `<storage_dir>`.
pub fn atomic_write(dst: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = dst.parent().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic_write: destination has no parent directory",
        ))
    })?;
    if !parent.exists() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(dst);
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dst)?;
    sync_parent_dir(parent);
    Ok(())
}
