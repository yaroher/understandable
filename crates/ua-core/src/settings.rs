//! Project-level configuration committed to the repository.
//!
//! Lives at `<project_root>/understandable.yaml` (or `.yml`). Every
//! subcommand looks it up before applying defaults so that two
//! engineers on the same repo get the same embedding model, the same
//! ignore list, the same incremental thresholds, etc.
//!
//! CLI flags always win; the config supplies fallbacks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;

pub const FILENAMES: &[&str] = &["understandable.yaml", "understandable.yml"];

/// Top-level config schema. Every section is optional — missing fields
/// fall back to the constructor defaults.
///
/// `deny_unknown_fields` is intentionally on: a typo like
/// `embeddings.providr: openai` should fail loudly with a typed serde
/// error rather than silently dropping the value. Forward-compat is
/// handled per-section by introducing new optional fields, never by
/// loosening this guard.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ProjectSettings {
    /// Schema version. Bumping it lets us migrate older files cleanly.
    pub version: u32,
    pub project: ProjectIdent,
    pub storage: StorageSettings,
    pub embeddings: EmbeddingSettings,
    pub llm: LlmSettings,
    pub ignore: IgnoreSettings,
    pub incremental: IncrementalSettings,
    pub dashboard: DashboardSettings,
    /// Hints for the README + reviewers — purely informational.
    pub git: GitSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct StorageSettings {
    /// Project-relative directory holding `graph*.db.zst`,
    /// `meta.json`, `config.json` and the agent intermediate / tmp
    /// folders. Default `.understandable`.
    pub dir: String,
    /// Stem of the canonical codebase database file (no extension).
    /// The persisted file is `<dir>/<db_name>.db.zst`. Domain and
    /// knowledge graphs land at `<dir>/<db_name>.domain.db.zst` and
    /// `<dir>/<db_name>.knowledge.db.zst` respectively.
    pub db_name: String,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            dir: ".understandable".into(),
            db_name: "graph".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ProjectIdent {
    /// Override the auto-detected project name.
    pub name: Option<String>,
    /// Free-form description copied into the graph's `project` block.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct EmbeddingSettings {
    /// One of `openai` / `ollama` / `local`.
    pub provider: String,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    /// Inputs per provider call (cap on the per-batch payload).
    pub batch_size: usize,
    /// Embed automatically as the last step of `analyze`.
    pub embed_on_analyze: bool,
    /// How many embedding batches to run in parallel during
    /// `understandable embed`. Default 2 — most providers throttle hard
    /// at higher rates and the per-batch payload is already substantial.
    /// Saturated at 1 if the user passes 0.
    pub concurrency: usize,
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: None,
            endpoint: None,
            batch_size: 32,
            embed_on_analyze: false,
            concurrency: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct LlmSettings {
    pub provider: String,
    pub model: Option<String>,
    pub max_files: usize,
    pub temperature: f32,
    /// Run `analyze --with-llm` automatically (no manual flag).
    pub run_on_analyze: bool,
    /// How many files to send to the LLM in parallel during
    /// `analyze --with-llm`. Default 4. Bump if your provider quota
    /// allows; lower if you keep getting 429s (retry helps but isn't
    /// free). Saturated at 1 if the user passes 0.
    pub concurrency: usize,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: None,
            max_files: 50,
            temperature: 0.2,
            run_on_analyze: false,
            concurrency: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct IgnoreSettings {
    /// Extra glob-ish path prefixes layered on top of `.gitignore` and
    /// `.understandignore`.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct IncrementalSettings {
    /// Hard limit — beyond this many changed files, `analyze
    /// --incremental` recommends a full rebuild.
    pub full_threshold: usize,
    /// Below this graph size the percentage check is skipped (so tiny
    /// projects don't trigger a full rebuild on every diff).
    pub big_graph_threshold: usize,
}

impl Default for IncrementalSettings {
    fn default() -> Self {
        Self {
            full_threshold: 30,
            big_graph_threshold: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct DashboardSettings {
    pub host: String,
    pub port: u16,
    pub auto_open: bool,
}

impl Default for DashboardSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 5173,
            auto_open: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", default, deny_unknown_fields)]
#[non_exhaustive]
pub struct GitSettings {
    /// True ⇒ the README/init prompt advises committing
    /// `.understandable/graph.db.zst` for 100 % reproducibility.
    pub commit_db: bool,
    /// True ⇒ keep the embedding tables alongside the graph file. (No
    /// effect at runtime — embeddings live in the same `.db.zst`.)
    pub commit_embeddings: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            commit_db: true,
            commit_embeddings: true,
        }
    }
}

impl ProjectSettings {
    /// Walk `project_root` for an `understandable.yaml` (or `.yml`)
    /// and parse it. Returns `Ok(None)` if no file exists.
    pub fn load(project_root: impl AsRef<Path>) -> Result<Option<Self>, Error> {
        if let Some(path) = Self::find(project_root) {
            let raw = std::fs::read_to_string(&path)?;
            let parsed: ProjectSettings = serde_yaml_ng::from_str(&raw)?;
            return Ok(Some(parsed));
        }
        Ok(None)
    }

    /// Same as [`load`] but substitutes the default settings when no
    /// file is on disk — every caller therefore gets a populated struct.
    pub fn load_or_default(project_root: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self::load(project_root)?.unwrap_or_else(Self::recommended))
    }

    /// Locate the settings file, regardless of which extension was used.
    pub fn find(project_root: impl AsRef<Path>) -> Option<PathBuf> {
        let root = project_root.as_ref();
        for name in FILENAMES {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    /// Default file path used when scaffolding a fresh config.
    pub fn default_path(project_root: impl AsRef<Path>) -> PathBuf {
        project_root.as_ref().join(FILENAMES[0])
    }

    /// Sensible defaults for a brand-new project — committed DB, default
    /// providers, version 1.
    pub fn recommended() -> Self {
        Self {
            version: 1,
            ..Self::default()
        }
    }

    pub fn to_yaml(&self) -> Result<String, Error> {
        Ok(serde_yaml_ng::to_string(self)?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_yaml()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let s = ProjectSettings::recommended();
        let yaml = s.to_yaml().unwrap();
        let s2: ProjectSettings = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn partial_yaml_uses_defaults() {
        let yaml = r#"
version: 1
embeddings:
  provider: ollama
  model: bge-m3
"#;
        let s: ProjectSettings = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(s.embeddings.provider, "ollama");
        assert_eq!(s.embeddings.model.as_deref(), Some("bge-m3"));
        // Untouched fields keep their defaults.
        assert_eq!(s.embeddings.batch_size, 32);
        assert_eq!(s.dashboard.port, 5173);
        assert_eq!(s.incremental.full_threshold, 30);
        assert!(s.git.commit_db);
    }

    #[test]
    fn unknown_field_emits_typed_error() {
        // Typo at the section level — `embeddings.providr` instead of
        // `provider` — must fail loudly. Earlier versions silently
        // accepted unknown fields, masking config bugs.
        let yaml = r#"
version: 1
embeddings:
  providr: openai
"#;
        let err = serde_yaml_ng::from_str::<ProjectSettings>(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("providr") || msg.contains("unknown field"),
            "expected typed unknown-field error, got: {msg}",
        );
    }

    #[test]
    fn forward_compat_unknown_at_top_level_fails() {
        // Symmetric case at the top level: a section name we don't know
        // about should surface rather than disappear.
        let yaml = r#"
version: 1
unknown_section: true
"#;
        let err = serde_yaml_ng::from_str::<ProjectSettings>(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown_section") || msg.contains("unknown field"),
            "expected typed unknown-field error, got: {msg}",
        );
    }
}
