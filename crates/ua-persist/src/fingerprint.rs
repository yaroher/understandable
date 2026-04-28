//! File fingerprinting via blake3.
//!
//! Two complementary digests live on every fingerprint:
//!   * [`Fingerprint::hash`] — blake3 of the raw file bytes. Flips on
//!     any whitespace / comment / body edit.
//!   * [`Fingerprint::structural_hash`] — blake3 of the parsed AST shape
//!     (sorted function/class/import/export names + arities). Stable
//!     under cosmetic edits, flips on signature changes. `None` when
//!     the language has no analyzer plugin or when analysis failed; the
//!     change classifier falls back to its regex heuristics in that
//!     case.

use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub path: String,
    pub hash: String,
    pub modified_at: Option<i64>,
    /// Optional second hash derived from the AST shape rather than the
    /// raw bytes. `None` for languages without an analyzer plugin or
    /// for legacy archives that pre-date this field — callers must
    /// fall back to the byte hash in that case.
    pub structural_hash: Option<String>,
}

/// Hash file contents with blake3 and return the hex digest.
pub fn blake3_file(path: impl AsRef<Path>) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Hash an in-memory byte slice. Convenience wrapper.
pub fn blake3_string(content: impl AsRef<[u8]>) -> String {
    blake3::hash(content.as_ref()).to_hex().to_string()
}

/// Read modtime as a unix epoch seconds value, ignoring failures.
pub fn modtime_secs(path: impl AsRef<Path>) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_secs() as i64)
}
