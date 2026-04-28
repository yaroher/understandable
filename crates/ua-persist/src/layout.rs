//! Project-relative storage layout. Default is
//! `<project_root>/.understandable/<db_name>.tar.zst`, but every piece
//! is configurable via `storage` in `understandable.yaml`.

use std::path::{Path, PathBuf};

use ua_core::{ProjectSettings, StorageSettings};

#[derive(Debug, Clone)]
pub struct ProjectLayout {
    /// Absolute path of the storage directory (e.g. `.../.understandable`).
    pub root: PathBuf,
    /// Filename stem for the canonical codebase DB. Domain / knowledge
    /// graphs land at `<root>/<db_name>.domain.tar.zst` and
    /// `<root>/<db_name>.knowledge.tar.zst`.
    pub db_name: String,
    /// Absolute path of the project this layout belongs to. Stamped
    /// into the `meta` table on `save_graph` so we can detect when two
    /// projects accidentally share the same `storage.dir` (e.g. an
    /// absolute cache path used by multiple repos). Optional only so
    /// legacy/test constructors that didn't supply one still work.
    pub project_root: Option<PathBuf>,
}

impl ProjectLayout {
    /// Build the default layout: `<project_root>/.understandable/graph*.tar.zst`.
    pub fn under(project_root: impl AsRef<Path>) -> Self {
        Self::with_storage(project_root, &StorageSettings::default())
    }

    /// Settings-aware layout. Reads `understandable.yaml` from
    /// `project_root` and applies its `storage` block; falls back to
    /// the defaults when no file is present. Use this in CLI
    /// subcommands so that custom `storage.dir` / `storage.db_name`
    /// from the YAML reach every code path automatically.
    pub fn for_project(project_root: impl AsRef<Path>) -> Self {
        let root = project_root.as_ref();
        match ProjectSettings::load_or_default(root) {
            Ok(s) => Self::with_storage(root, &s.storage),
            Err(_) => Self::under(root),
        }
    }

    /// Build the layout from a settings block. The `dir` is resolved
    /// relative to `project_root` when it isn't absolute.
    pub fn with_storage(project_root: impl AsRef<Path>, storage: &StorageSettings) -> Self {
        let project_root_path = project_root.as_ref().to_path_buf();
        let dir_path = Path::new(&storage.dir);
        let root = if dir_path.is_absolute() {
            dir_path.to_path_buf()
        } else {
            project_root_path.join(&storage.dir)
        };
        let db_name = if storage.db_name.is_empty() {
            "graph".to_string()
        } else {
            storage.db_name.clone()
        };
        Self {
            root,
            db_name,
            project_root: Some(canonicalize_lossy(&project_root_path)),
        }
    }

    /// Path of the canonical codebase archive
    /// (`<root>/<db_name>.tar.zst`).
    pub fn graph_archive(&self) -> PathBuf {
        self.root.join(format!("{}.tar.zst", self.db_name))
    }

    /// Per-kind archive — codebase / domain / knowledge graphs land
    /// side-by-side in the same dir. Suffix mirrors the legacy zst
    /// path so existing tooling that scans `<storage_dir>` keeps
    /// working with one rename.
    pub fn graph_archive_for(&self, kind: &str) -> PathBuf {
        match kind {
            "codebase" | "" => self.graph_archive(),
            other => self.root.join(format!("{}.{other}.tar.zst", self.db_name)),
        }
    }

    /// Backwards-compatible accessor — the canonical codebase archive.
    /// Old name kept for callers that haven't migrated yet (mostly
    /// tests).
    #[deprecated(since = "0.2.0", note = "use `graph_archive` instead")]
    pub fn graph_db_zst(&self) -> PathBuf {
        self.graph_archive()
    }

    /// Backwards-compatible accessor — the per-kind archive.
    #[deprecated(since = "0.2.0", note = "use `graph_archive_for` instead")]
    pub fn graph_db_zst_for(&self, kind: &str) -> PathBuf {
        self.graph_archive_for(kind)
    }

    pub fn meta_json(&self) -> PathBuf {
        self.root.join("meta.json")
    }

    pub fn config_json(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn intermediate_dir(&self) -> PathBuf {
        self.root.join("intermediate")
    }

    /// Project-relative `.understandignore` (peer of `.gitignore`).
    pub fn understandignore(project_root: impl AsRef<Path>) -> PathBuf {
        project_root.as_ref().join(".understandignore")
    }

    /// Make sure the layout directory exists.
    pub fn ensure_exists(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)
    }

    /// String form of the project root that gets stamped into `meta`.
    /// Returns `None` for legacy layouts that didn't record one.
    pub fn project_root_stamp(&self) -> Option<String> {
        self.project_root
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

/// Canonicalise the path; fall back to the original if the filesystem
/// can't resolve it (e.g. a project root that doesn't exist on disk —
/// rare, but seen in some unit tests). Trailing slashes are stripped
/// so `/repo` and `/repo/` compare equal.
fn canonicalize_lossy(p: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // Strip a trailing separator. PathBuf doesn't normalise this.
    let s = resolved.to_string_lossy();
    let trimmed = s.trim_end_matches('/').trim_end_matches('\\');
    if trimmed.len() == s.len() {
        resolved
    } else {
        PathBuf::from(trimmed)
    }
}
