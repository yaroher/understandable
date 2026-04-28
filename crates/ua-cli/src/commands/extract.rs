//! `understandable extract` — replaces the original `extract-structure.mjs`.
//!
//! Reads a JSON batch describing a set of source files and writes a JSON
//! output with each file's `StructuralAnalysis`, call graph, and basic
//! line metrics.
//!
//! # Input shape
//!
//! Two shapes are accepted (untagged enum dispatch):
//!
//! 1. **Legacy minimal** (the Rust-native shape that existed before the
//!    TS-compat work):
//!    ```json
//!    { "files": [{ "path": "src/foo.ts", "language": "typescript",
//!                  "content": "..." }] }
//!    ```
//!    `content` is optional — when absent, the file is read from disk.
//!
//! 2. **TS-compat** (matches `extract-structure.mjs` from the upstream
//!    Understand-Anything plugin):
//!    ```json
//!    {
//!      "projectRoot": "/abs/path/to/project",
//!      "batchFiles": [
//!        { "path": "src/auth.ts", "language": "ts",
//!          "sizeLines": 234, "fileCategory": "source" }
//!      ],
//!      "batchImportData": { "src/auth.ts": ["src/db.ts", ...] }
//!    }
//!    ```
//!    - Relative `path` entries are resolved against `projectRoot`.
//!    - `language` is optional; missing values are inferred from the
//!      file extension via `LanguageRegistry::for_path`.
//!    - `sizeLines` acts as a budget hint: any file whose claimed size
//!      exceeds [`MAX_SIZE_LINES`] is skipped to protect the LLM token
//!      budget downstream — unless `--force` is passed.
//!    - `fileCategory` is captured into per-file `metadata` so the
//!      caller can keep the value flowing through the pipeline.
//!    - `batchImportData` is captured wholesale into per-file
//!      `metadata.batch_import_data` (the resolved list for that file)
//!      until import-resolution wiring lands.
//!
//! The output schema is unchanged regardless of which input shape is
//! used; downstream agents already consume it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use serde::{Deserialize, Serialize};
use ua_extract::{default_registry, LanguageRegistry};

/// Files claiming more than this many lines are skipped in TS-compat
/// mode unless `--force` is passed. Mirrors the upstream protection
/// against feeding huge files into a downstream LLM.
const MAX_SIZE_LINES: u64 = 5000;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Input JSON describing the files to analyse.
    #[arg(long)]
    pub batch: PathBuf,
    /// Output JSON path. `-` writes to stdout.
    #[arg(long, default_value = "-")]
    pub out: PathBuf,
    /// Bypass the `sizeLines > MAX_SIZE_LINES` skip gate (TS-compat only).
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

// ---------------------------------------------------------------------------
// Input deserialisers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExtractRequest {
    /// TS-style shape with project root + per-file metadata. Listed
    /// first because `serde(untagged)` tries variants in order and the
    /// TS shape has more required fields, making it less likely to
    /// match accidentally.
    TsCompat {
        #[serde(rename = "projectRoot")]
        project_root: PathBuf,
        #[serde(rename = "batchFiles")]
        batch_files: Vec<TsBatchFile>,
        #[serde(rename = "batchImportData", default)]
        batch_import_data: Option<serde_json::Value>,
    },
    /// Legacy minimal shape (the prior Rust-native contract).
    Minimal { files: Vec<MinimalFile> },
}

#[derive(Debug, Deserialize)]
struct MinimalFile {
    path: String,
    language: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TsBatchFile {
    path: PathBuf,
    #[serde(default)]
    language: Option<String>,
    #[serde(rename = "sizeLines", default)]
    size_lines: Option<u64>,
    #[serde(rename = "fileCategory", default)]
    file_category: Option<String>,
}

/// Normalised, internal per-file extraction job. Both input shapes
/// funnel into this so the extraction loop only has to deal with one
/// representation.
#[derive(Debug)]
struct ExtractJob {
    /// Path string used in node IDs / output. For TS-compat this is
    /// the relative path as supplied by the caller.
    display_path: String,
    /// Absolute path to read from disk (if `content` is `None`).
    fs_path: PathBuf,
    /// Optional language id. If `None`, detected from extension.
    language: Option<String>,
    /// Optional pre-supplied content (legacy shape only).
    content: Option<String>,
    /// Optional pre-declared line count (TS-compat only).
    size_lines: Option<u64>,
    /// Optional file category (TS-compat only).
    file_category: Option<String>,
    /// Optional pass-through import data for this file (TS-compat only).
    /// Captured into the output's per-file `metadata`.
    /// TODO: wire batchImportData into import resolution.
    batch_import_data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Output schema
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct BatchOutput {
    script_completed: bool,
    files_analyzed: usize,
    files_skipped: usize,
    results: Vec<FileResult>,
}

#[derive(Debug, Serialize)]
struct FileResult {
    path: String,
    language: String,
    total_lines: usize,
    nonempty_lines: usize,
    analysis: ua_core::StructuralAnalysis,
    call_graph: Vec<ua_core::CallGraphEntry>,
    /// Free-form forward-compatible metadata bag. Currently used to
    /// surface `file_category` and any per-file `batch_import_data`
    /// that we do not yet consume natively.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    metadata: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(args: Args) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&args.batch)?;
    let request: ExtractRequest = serde_json::from_str(&raw)?;
    let lang_registry = LanguageRegistry::default_registry();

    let (jobs, gate_skipped) = match request {
        ExtractRequest::Minimal { files } => (jobs_from_minimal(files), 0usize),
        ExtractRequest::TsCompat {
            project_root,
            batch_files,
            batch_import_data,
        } => jobs_from_ts_compat(project_root, batch_files, batch_import_data, args.force),
    };

    let registry = default_registry();
    let mut results = Vec::with_capacity(jobs.len());
    let mut skipped = gate_skipped;

    for job in jobs {
        let content = match job.content.clone() {
            Some(c) => c,
            None => match std::fs::read_to_string(&job.fs_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %job.display_path,
                        fs_path = %job.fs_path.display(),
                        error = %e,
                        "skipping unreadable file",
                    );
                    skipped += 1;
                    continue;
                }
            },
        };

        // Resolve language: explicit > extension lookup.
        let language = match job.language.clone() {
            Some(l) => l,
            None => match lang_registry.for_path(Path::new(&job.display_path)) {
                Some(cfg) => cfg.id.clone(),
                None => {
                    tracing::warn!(
                        path = %job.display_path,
                        "skipping file with unknown language",
                    );
                    skipped += 1;
                    continue;
                }
            },
        };

        let total_lines = content.lines().count();
        let nonempty_lines = content.lines().filter(|l| !l.trim().is_empty()).count();

        let analysis = match registry.analyze_file(&language, &job.display_path, &content) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(
                    path = %job.display_path,
                    lang = %language,
                    error = %e,
                    "extraction failed",
                );
                skipped += 1;
                continue;
            }
        };
        let call_graph = registry
            .extract_call_graph(&language, &job.display_path, &content)
            .unwrap_or_default();

        let mut metadata = BTreeMap::new();
        if let Some(category) = job.file_category {
            metadata.insert(
                "file_category".to_string(),
                serde_json::Value::String(category),
            );
        }
        if let Some(size) = job.size_lines {
            metadata.insert(
                "declared_size_lines".to_string(),
                serde_json::Value::Number(size.into()),
            );
        }
        if let Some(import_data) = job.batch_import_data {
            // TODO: wire batchImportData into import resolution.
            metadata.insert("batch_import_data".to_string(), import_data);
        }

        results.push(FileResult {
            path: job.display_path,
            language,
            total_lines,
            nonempty_lines,
            analysis,
            call_graph,
            metadata,
        });
    }

    let out = BatchOutput {
        script_completed: true,
        files_analyzed: results.len(),
        files_skipped: skipped,
        results,
    };
    let json = serde_json::to_string_pretty(&out)?;
    write_output(&args.out, &json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Input -> jobs translation
// ---------------------------------------------------------------------------

fn jobs_from_minimal(files: Vec<MinimalFile>) -> Vec<ExtractJob> {
    files
        .into_iter()
        .map(|f| ExtractJob {
            fs_path: PathBuf::from(&f.path),
            display_path: f.path,
            language: Some(f.language),
            content: f.content,
            size_lines: None,
            file_category: None,
            batch_import_data: None,
        })
        .collect()
}

fn jobs_from_ts_compat(
    project_root: PathBuf,
    batch_files: Vec<TsBatchFile>,
    batch_import_data: Option<serde_json::Value>,
    force: bool,
) -> (Vec<ExtractJob>, usize) {
    let mut jobs = Vec::with_capacity(batch_files.len());
    let mut skipped = 0usize;

    for file in batch_files {
        // Skip absurdly large files unless the caller forced it.
        if let Some(size) = file.size_lines {
            if size > MAX_SIZE_LINES && !force {
                tracing::warn!(
                    path = %file.path.display(),
                    size_lines = size,
                    threshold = MAX_SIZE_LINES,
                    "skipping oversized file (pass --force to override)",
                );
                skipped += 1;
                continue;
            }
        }

        let display_path = file.path.to_string_lossy().into_owned();
        let fs_path = if file.path.is_absolute() {
            file.path.clone()
        } else {
            project_root.join(&file.path)
        };

        // Pull the per-file slice of the import map, if any. We hand
        // it off as opaque JSON until the extractor itself learns to
        // consume it.
        let per_file_import_data = batch_import_data
            .as_ref()
            .and_then(|v| v.get(&display_path))
            .cloned();

        jobs.push(ExtractJob {
            display_path,
            fs_path,
            language: file.language,
            content: None,
            size_lines: file.size_lines,
            file_category: file.file_category,
            batch_import_data: per_file_import_data,
        });
    }

    (jobs, skipped)
}

fn write_output(path: &PathBuf, content: &str) -> anyhow::Result<()> {
    if path == &PathBuf::from("-") {
        use std::io::Write;
        std::io::stdout().write_all(content.as_bytes())?;
        std::io::stdout().write_all(b"\n")?;
    } else {
        std::fs::write(path, content)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(input: serde_json::Value) -> ExtractRequest {
        serde_json::from_value(input).expect("input parses")
    }

    #[test]
    fn parses_minimal_legacy_input() {
        let req = parse(json!({
            "files": [
                { "path": "src/a.rs", "language": "rust" },
                { "path": "src/b.rs", "language": "rust", "content": "fn main(){}" }
            ]
        }));
        match req {
            ExtractRequest::Minimal { files } => {
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].path, "src/a.rs");
                assert_eq!(files[0].language, "rust");
                assert!(files[0].content.is_none());
                assert_eq!(files[1].content.as_deref(), Some("fn main(){}"));
            }
            ExtractRequest::TsCompat { .. } => panic!("expected Minimal variant"),
        }
    }

    #[test]
    fn parses_ts_compat_input_with_all_fields() {
        let req = parse(json!({
            "projectRoot": "/repo",
            "batchFiles": [
                {
                    "path": "src/auth.ts",
                    "language": "ts",
                    "sizeLines": 234,
                    "fileCategory": "source"
                }
            ],
            "batchImportData": {
                "src/auth.ts": ["src/db.ts"]
            }
        }));
        match req {
            ExtractRequest::TsCompat {
                project_root,
                batch_files,
                batch_import_data,
            } => {
                assert_eq!(project_root, PathBuf::from("/repo"));
                assert_eq!(batch_files.len(), 1);
                assert_eq!(batch_files[0].path, PathBuf::from("src/auth.ts"));
                assert_eq!(batch_files[0].language.as_deref(), Some("ts"));
                assert_eq!(batch_files[0].size_lines, Some(234));
                assert_eq!(batch_files[0].file_category.as_deref(), Some("source"));
                let import_data = batch_import_data.expect("import data present");
                assert_eq!(import_data.get("src/auth.ts"), Some(&json!(["src/db.ts"])));
            }
            ExtractRequest::Minimal { .. } => panic!("expected TsCompat variant"),
        }
    }

    #[test]
    fn parses_ts_compat_input_with_optional_fields_omitted() {
        let req = parse(json!({
            "projectRoot": "/repo",
            "batchFiles": [
                { "path": "src/auth.ts" }
            ]
        }));
        match req {
            ExtractRequest::TsCompat {
                project_root,
                batch_files,
                batch_import_data,
            } => {
                assert_eq!(project_root, PathBuf::from("/repo"));
                assert_eq!(batch_files.len(), 1);
                assert_eq!(batch_files[0].path, PathBuf::from("src/auth.ts"));
                assert!(batch_files[0].language.is_none());
                assert!(batch_files[0].size_lines.is_none());
                assert!(batch_files[0].file_category.is_none());
                assert!(batch_import_data.is_none());
            }
            ExtractRequest::Minimal { .. } => panic!("expected TsCompat variant"),
        }
    }

    #[test]
    fn relative_paths_resolve_against_project_root() {
        let project_root = PathBuf::from("/tmp/proj");
        let batch = vec![
            TsBatchFile {
                path: PathBuf::from("src/a.rs"),
                language: Some("rust".into()),
                size_lines: None,
                file_category: None,
            },
            TsBatchFile {
                path: PathBuf::from("/abs/elsewhere/b.rs"),
                language: Some("rust".into()),
                size_lines: None,
                file_category: None,
            },
        ];
        let (jobs, skipped) = jobs_from_ts_compat(project_root.clone(), batch, None, false);
        assert_eq!(skipped, 0);
        assert_eq!(jobs.len(), 2);

        // Relative path: joined onto project_root.
        assert_eq!(jobs[0].fs_path, project_root.join("src/a.rs"));
        // display_path stays as the caller's input.
        assert_eq!(jobs[0].display_path, "src/a.rs");

        // Absolute path: untouched.
        assert_eq!(jobs[1].fs_path, PathBuf::from("/abs/elsewhere/b.rs"));
        assert_eq!(jobs[1].display_path, "/abs/elsewhere/b.rs");
    }

    #[test]
    fn size_lines_above_threshold_skips_file_unless_force() {
        let project_root = PathBuf::from("/tmp/proj");
        let make_batch = || {
            vec![
                TsBatchFile {
                    path: PathBuf::from("small.rs"),
                    language: Some("rust".into()),
                    size_lines: Some(100),
                    file_category: None,
                },
                TsBatchFile {
                    path: PathBuf::from("huge.rs"),
                    language: Some("rust".into()),
                    size_lines: Some(MAX_SIZE_LINES + 1),
                    file_category: None,
                },
            ]
        };

        // Without force: huge.rs is gated out.
        let (jobs, skipped) = jobs_from_ts_compat(project_root.clone(), make_batch(), None, false);
        assert_eq!(skipped, 1);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].display_path, "small.rs");

        // With force: huge.rs is kept.
        let (jobs, skipped) = jobs_from_ts_compat(project_root, make_batch(), None, true);
        assert_eq!(skipped, 0);
        assert_eq!(jobs.len(), 2);
    }

    #[test]
    fn batch_import_data_is_split_per_file() {
        let project_root = PathBuf::from("/tmp/proj");
        let batch = vec![TsBatchFile {
            path: PathBuf::from("src/a.rs"),
            language: Some("rust".into()),
            size_lines: None,
            file_category: Some("code".into()),
        }];
        let import_data = json!({
            "src/a.rs": ["src/b.rs", "src/c.rs"],
            "src/other.rs": ["src/d.rs"]
        });
        let (jobs, _) = jobs_from_ts_compat(project_root, batch, Some(import_data), false);
        assert_eq!(jobs.len(), 1);
        assert_eq!(
            jobs[0].batch_import_data,
            Some(json!(["src/b.rs", "src/c.rs"]))
        );
        assert_eq!(jobs[0].file_category.as_deref(), Some("code"));
    }
}
