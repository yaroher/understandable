//! `understandable analyze` — full or incremental analysis pipeline.
//!
//! With `--incremental`, the binary uses stored fingerprints to detect
//! changed files and rebuilds only the affected slice of the graph;
//! with `--plan-only`, it skips persistence and prints a JSON change
//! plan (consumed by the post-commit hook).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Args as ClapArgs;
use serde::Serialize;
use ua_analyzer::{
    classify_change_with, detect_layers, generate_heuristic_tour,
    ChangeLevel as ClassifierLevel, FileMeta, GraphBuilder,
};
use ua_core::{Complexity, GraphNode, KnowledgeGraph, NodeType, ProjectSettings};
use ua_extract::{default_registry, FrameworkRegistry, LanguageRegistry, PluginRegistry};
use ua_llm::{
    file_summary_prompts, parse_file_summary, AnthropicClient, CompleteRequest,
    FileSummaryRequest,
};
use ua_persist::{
    blake3_file, blake3_string, fingerprint::modtime_secs, walk_project, Fingerprint,
    IgnoreFilter, ProjectLayout, Storage,
};

use crate::commands::usage::TokenTotals;
use crate::util::time::iso8601_now;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Force a full rebuild even if a graph already exists.
    #[arg(long)]
    pub full: bool,
    /// Skip files whose blake3 hash already matches the stored
    /// fingerprint, and only rebuild the slice of the graph that
    /// covers files with changed content.
    #[arg(long, conflicts_with = "full")]
    pub incremental: bool,
    /// Run the analyzer in dry-run mode and print a JSON change plan
    /// to stdout. Used by the post-commit hook to decide whether to
    /// invoke the LLM at all.
    #[arg(long, requires = "incremental")]
    pub plan_only: bool,
    /// Emit a project-scanner JSON document compatible with the TS
    /// `project-scanner` agent and exit. Schema:
    ///
    /// ```json
    /// {
    ///   "name": "<project name>",
    ///   "languages": ["rust", "typescript", ...],
    ///   "frameworks": ["React", "Axum", ...],
    ///   "files": [{ "path": "src/foo.rs", "fileCategory": "source" }, ...],
    ///   "totalFiles": 42,
    ///   "filteredByIgnore": 7,
    ///   "estimatedComplexity": "small|medium|large",
    ///   "importMap": {},
    ///   "rawDescription": "...",
    ///   "readmeHead": "...",
    ///   "scriptCompleted": true
    /// }
    /// ```
    ///
    /// `fileCategory` is heuristic: `source` for code files, `test`
    /// for paths matching `*test*` / `*spec*`, `config` for known
    /// config names, `doc` for `.md`/`.rst`, `data` for `.json`/
    /// `.yaml` / `.yml`. Unknown buckets fall back to `source`.
    #[arg(long, conflicts_with_all = ["plan_only", "incremental", "with_llm"])]
    pub scan_only: bool,
    /// Print a richer one-line summary of what changed after analyze
    /// finishes (file counts, deletions, reanalysed paths). Used by
    /// the post-commit hook for human-readable feedback.
    #[arg(long)]
    pub review: bool,
    /// After analyze, write `.understandable/auto-update.signal` to
    /// nudge the IDE-side agents to refresh. CLI flag wins over any
    /// project default.
    #[arg(long, conflicts_with = "no_auto_update")]
    pub auto_update: bool,
    /// Suppress the auto-update nudge file even if a project default
    /// would have written it.
    #[arg(long)]
    pub no_auto_update: bool,
    /// Project name (default: `project.name` from settings, or the
    /// directory name).
    #[arg(long)]
    pub name: Option<String>,
    /// After extracting structure, ask the configured LLM provider
    /// for per-file summary/tags/complexity. Requires
    /// `ANTHROPIC_API_KEY` when `llm.provider == "anthropic"`. With
    /// `llm.provider == "host"` the per-file call is delegated to
    /// the IDE plugin (markdown agents) and this flag becomes a
    /// no-op for the binary.
    #[arg(long)]
    pub with_llm: bool,
    /// Override the LLM model (default: `llm.model` from settings,
    /// or `claude-opus-4-7`).
    #[arg(long)]
    pub llm_model: Option<String>,
    /// Cap how many files get sent to the LLM. `0` (the default)
    /// means "use `llm.max_files` from settings, or 50".
    #[arg(long, default_value_t = 0)]
    pub llm_max_files: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum ChangeLevel {
    None,
    Cosmetic,
    Structural,
}

impl From<ClassifierLevel> for ChangeLevel {
    fn from(level: ClassifierLevel) -> Self {
        match level {
            ClassifierLevel::None => ChangeLevel::None,
            ClassifierLevel::Cosmetic => ChangeLevel::Cosmetic,
            ClassifierLevel::Structural => ChangeLevel::Structural,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Action {
    Skip,
    PartialUpdate,
    ArchitectureUpdate,
    FullUpdate,
}

#[derive(Debug, Serialize)]
struct FileChange {
    file_path: String,
    change_level: ChangeLevel,
    details: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ChangePlan {
    action: Action,
    reason: String,
    files_to_reanalyze: Vec<String>,
    rerun_architecture: bool,
    rerun_tour: bool,
    file_changes: Vec<FileChange>,
}

pub async fn run(mut args: Args, project_path: &Path) -> anyhow::Result<()> {
    // Settings drive both the file walker (`ignore.paths`) and the
    // post-analyze auto-embed step. Loaded once so both paths use the
    // same snapshot.
    let settings = ProjectSettings::load_or_default(project_path)?;

    // Project name precedence: CLI `--name` > settings.project.name >
    // directory basename. Settings file is the canonical source for
    // the team-wide name, and the CLI flag is a per-invocation
    // override.
    let project_name = resolve_project_name(args.name.as_deref(), &settings, project_path);
    let git_hash =
        ua_persist::staleness::current_git_head(project_path).unwrap_or_default();

    // `--scan-only` is a one-shot dump of the project-scanner JSON
    // shape used by the TS agent. It deliberately bypasses the
    // analyzer pipeline (no fingerprints, no graph) so it can serve
    // as a cheap preview.
    if args.scan_only {
        return run_scan_only(project_path, &project_name, &settings).await;
    }

    // `llm.run_on_analyze` lets the YAML config opt every analyze run
    // into per-file LLM enrichment without a flag. The CLI flag still
    // wins (explicit `--with-llm` always enables; absence + setting
    // == enable).
    if !args.with_llm && settings.llm.run_on_analyze {
        args.with_llm = true;
    }

    if args.incremental {
        return run_incremental(args, project_path, &project_name, &git_hash, &settings).await;
    }

    tracing::info!(
        target: "ua_cli::analyze",
        project = %project_name,
        path = %project_path.display(),
        review = args.review,
        full = args.full,
        "starting full analysis"
    );

    // 1. Walk the project, honouring .gitignore + .understandignore +
    //    `ignore.paths`. The walker now plumbs the extra paths through
    //    `IgnoreFilter::extra_ignore_paths` so the underlying `ignore`
    //    crate prunes excluded subtrees natively (previously a post-walk
    //    filter swept them out, which still walked the whole tree).
    let filter = IgnoreFilter {
        extra_ignore_paths: settings.ignore.paths.clone(),
        ..IgnoreFilter::default()
    };
    let files: Vec<PathBuf> = walk_project(project_path, &filter).collect();
    tracing::info!(count = files.len(), "files discovered");

    // 2. Compute per-file metadata. Without `--with-llm` the summary and
    //    tags stay empty; with it, Anthropic gets a one-shot prompt per
    //    file and we cap the call count at `llm_max_files` so a runaway
    //    `understandable analyze --with-llm` on a giant repo can't melt
    //    the user's API budget. The LLM step needs the storage handle
    //    so it can consult / update the per-file response cache, so we
    //    open it before the call and reuse it for the persistence step
    //    further down.
    let layout = ProjectLayout::for_project(project_path);
    layout.ensure_exists()?;
    let storage = Arc::new(Storage::open(&layout).await?);

    let project_name_clone = project_name.clone();
    let llm_metas = if args.with_llm {
        if settings.llm.provider.eq_ignore_ascii_case("host") {
            // Host-provider mode: the IDE plugin runs the file-summariser
            // via the markdown agents. The binary has no client to
            // instantiate, so we log loudly and skip — better than
            // silently invoking Anthropic against the user's wishes.
            tracing::warn!(
                target: "ua_cli::analyze",
                "llm.provider == \"host\"; --with-llm is delegated to the IDE-side agents and the binary will skip the per-file LLM loop"
            );
            None
        } else {
            // Resolve model/cap/temperature with the documented
            // precedence (CLI > settings > library default).
            let model = args
                .llm_model
                .clone()
                .or_else(|| settings.llm.model.clone());
            let cap = resolve_llm_max_files(args.llm_max_files, &settings);
            Some(
                run_llm_summaries(
                    &project_name_clone,
                    project_path,
                    &files,
                    model,
                    cap,
                    settings.llm.concurrency.max(1),
                    settings.llm.temperature,
                    storage.clone(),
                )
                .await?,
            )
        }
    } else {
        None
    };

    let mut builder = GraphBuilder::new(project_name.clone(), git_hash);
    for file in &files {
        let rel = relative(file, project_path);
        let meta = llm_metas
            .as_ref()
            .and_then(|m| m.get(&rel))
            .cloned()
            .unwrap_or_else(|| FileMeta {
                summary: String::new(),
                tags: Vec::new(),
                complexity: Complexity::Moderate,
            });
        builder.add_file(&rel, meta);
    }
    let now = iso8601_now();
    let mut graph = builder.build(now);
    // `project.description` is purely informational metadata, but it
    // surfaces in the dashboard / docs export. Pull it from the
    // settings file so a team can edit it once and have every
    // analyze run pick it up.
    if let Some(desc) = settings.project.description.as_deref() {
        graph.project.description = desc.to_string();
    }
    graph.layers = detect_layers(&graph);
    graph.tour = generate_heuristic_tour(&graph);

    // 3. Persist + refresh fingerprints in the same DB. `storage` was
    //    opened up front so the LLM step could share it; we just reuse
    //    the same handle here to avoid the second tar.zst round-trip.
    storage.save_graph(&graph).await?;
    let prints = compute_fingerprints(project_path, &files);
    storage.write_fingerprints(&prints).await?;
    storage.save(&layout).await?;
    cleanup_scratch_dirs(&layout);
    println!(
        "analysis complete: {} files → {} nodes, {} edges, {} layers, {} tour steps",
        files.len(),
        graph.nodes.len(),
        graph.edges.len(),
        graph.layers.len(),
        graph.tour.len(),
    );
    if args.review {
        // `--review`: extra one-line summary intended for the
        // post-commit hook's terminal output. The full-analyze path
        // re-reads every file, so "reanalysed" == "scanned" and the
        // delete count is implicit (full rebuild = none preserved).
        println!(
            "review: {} files scanned, {} nodes, {} layers (full rebuild — see `analyze --incremental --review` for diff-only summary)",
            files.len(),
            graph.nodes.len(),
            graph.layers.len(),
        );
    }

    // `--auto-update` writes a sentinel that downstream tooling
    // (post-commit hook, IDE plugins) watches for to know the graph
    // was refreshed. `--no-auto-update` suppresses it. Absent both
    // flags we leave the file alone.
    handle_auto_update_flag(args.auto_update, args.no_auto_update, project_path);

    // Honour `embeddings.embed_on_analyze`: kicking off the embed step
    // automatically saves the user from remembering to run
    // `understandable embed` after every full analyze. Embed errors are
    // logged but don't fail the analyze call — partial state on disk is
    // still useful for non-semantic search paths.
    if settings.embeddings.embed_on_analyze {
        if let Err(e) = run_auto_embed(project_path).await {
            tracing::warn!(error = %e, "embed_on_analyze step failed; run `understandable embed` manually");
        }
    }
    Ok(())
}

/// Convenience wrapper that runs `commands::embed::run` with the
/// defaults pulled from the project settings — used by the
/// `embed_on_analyze` hook so analyze can finish the pipeline.
async fn run_auto_embed(project_path: &Path) -> anyhow::Result<()> {
    let embed_args = crate::commands::embed::Args {
        embed_provider: None,
        embed_model: None,
        embed_endpoint: None,
        reset: false,
        force: false,
        batch_size: None,
    };
    crate::commands::embed::run(embed_args, project_path).await
}

async fn run_incremental(
    args: Args,
    project_path: &Path,
    project_name: &str,
    git_hash: &str,
    settings: &ProjectSettings,
) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project_path);
    let storage = Storage::open(&layout).await?;
    let mut graph = storage.load_graph().await?;
    let stored_prints: Vec<Fingerprint> = storage.read_fingerprints().await?;
    let stored_by_path: BTreeMap<String, Fingerprint> =
        stored_prints.iter().map(|f| (f.path.clone(), f.clone())).collect();

    let filter = IgnoreFilter {
        extra_ignore_paths: settings.ignore.paths.clone(),
        ..IgnoreFilter::default()
    };
    let files: Vec<PathBuf> = walk_project(project_path, &filter).collect();

    let mut new_prints: Vec<Fingerprint> = Vec::with_capacity(files.len());
    let mut current_paths: BTreeSet<String> = BTreeSet::new();
    let mut changes: Vec<FileChange> = Vec::new();
    let mut to_reanalyze: Vec<String> = Vec::new();
    let registry = LanguageRegistry::default_registry();
    let plugin_registry = default_registry();

    for path in &files {
        let rel = relative(path, project_path);
        current_paths.insert(rel.clone());
        let hash = match blake3_file(path) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let modified_at = modtime_secs(path);

        // Hash short-circuit: identical bytes ⇒ no change. Saves the
        // `git show` round-trip plus a regex pass for unchanged files,
        // which dominate the typical post-commit diff.
        let prior_print = stored_by_path.get(&rel);
        let language = registry
            .for_path(path)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        // Read content once. The classifier needs it; the structural
        // hasher reuses the same string so we don't re-IO. `None` here
        // means "couldn't read working-tree copy" — we still produce a
        // fingerprint with a byte hash, but the structural side stays
        // empty and the classifier defaults to Structural.
        let new_content: Option<String> = std::fs::read_to_string(path).ok();
        let level: ChangeLevel = match prior_print {
            None => {
                // New file (no prior fingerprint) is unconditionally
                // structural — there's no "old" content to diff against.
                ChangeLevel::Structural
            }
            Some(prev) if prev.hash == hash => ChangeLevel::None,
            Some(_) => {
                // Hashes differ → ask the classifier. The plugin
                // registry plumbs the structural-hash fast path, so a
                // whitespace-only edit on a Rust file lands as Cosmetic
                // without the regex collectors needing to model the
                // language. Parser failures fall back to the regex
                // tier inside `classify_change_with`.
                if new_content.is_none() {
                    tracing::warn!(
                        target: "ua_cli::analyze",
                        path = %rel,
                        "could not read working-tree copy; defaulting to structural"
                    );
                }
                let old_content = read_old_content(project_path, &rel);
                if new_content.is_none() || old_content.is_none() {
                    if old_content.is_none() {
                        tracing::warn!(
                            target: "ua_cli::analyze",
                            path = %rel,
                            "could not load prior content (git show failed); defaulting to structural"
                        );
                    }
                    ChangeLevel::Structural
                } else {
                    classify_change_with(
                        &plugin_registry,
                        &language,
                        &rel,
                        old_content.as_deref(),
                        new_content.as_deref(),
                    )
                    .into()
                }
            }
        };

        match level {
            ChangeLevel::Structural => {
                to_reanalyze.push(rel.clone());
                changes.push(FileChange {
                    file_path: rel.clone(),
                    change_level: level,
                    details: vec![if prior_print.is_some() {
                        "structural change".into()
                    } else {
                        "new file".into()
                    }],
                });
            }
            ChangeLevel::Cosmetic => {
                // Cosmetic: fingerprint advances so we don't re-classify
                // every run, but we don't reanalyse — the existing summary
                // is still correct.
                changes.push(FileChange {
                    file_path: rel.clone(),
                    change_level: level,
                    details: vec!["cosmetic change (whitespace/comments only)".into()],
                });
            }
            ChangeLevel::None => {}
        }

        // Reuse the in-memory body for the structural hash so we don't
        // re-read the file. When we couldn't load the body at all, fall
        // back to `None` — the classifier already defaulted to
        // Structural, so a missing structural hash here doesn't lose
        // any signal.
        let structural_hash = compute_structural_hash(
            &registry,
            &plugin_registry,
            path,
            &rel,
            new_content.as_deref(),
        );

        new_prints.push(Fingerprint {
            path: rel,
            hash,
            modified_at,
            structural_hash,
        });
    }

    // Deletions: in the stored set but missing from disk.
    let mut deleted: Vec<String> = Vec::new();
    for stored in &stored_prints {
        if !current_paths.contains(&stored.path) {
            deleted.push(stored.path.clone());
            changes.push(FileChange {
                file_path: stored.path.clone(),
                change_level: ChangeLevel::Structural,
                details: vec!["file deleted".into()],
            });
        }
    }

    // Decide on the action. The hard threshold (`incremental.full_threshold`,
    // default 30) avoids recommending a full rebuild on tiny projects;
    // the percentage check only kicks in once the graph crosses
    // `incremental.big_graph_threshold` (default 50). Both knobs are
    // tunable via `understandable.yaml` so a monorepo can dial them up
    // without touching the binary.
    let total_changes = to_reanalyze.len() + deleted.len();
    let big_graph = graph.nodes.len() >= settings.incremental.big_graph_threshold;
    let action = if total_changes == 0 {
        Action::Skip
    } else if total_changes > settings.incremental.full_threshold
        || (big_graph && total_changes * 2 > graph.nodes.len())
    {
        Action::FullUpdate
    } else if any_directory_change(&deleted, &to_reanalyze, &graph) {
        Action::ArchitectureUpdate
    } else {
        Action::PartialUpdate
    };

    let plan = ChangePlan {
        action,
        reason: match action {
            Action::Skip => "no source files changed".into(),
            Action::FullUpdate => format!("{total_changes} files changed (recommend `--full`)"),
            Action::ArchitectureUpdate => {
                format!("{total_changes} files changed in new directories")
            }
            Action::PartialUpdate => {
                format!(
                    "{} reanalyse + {} delete",
                    to_reanalyze.len(),
                    deleted.len()
                )
            }
        },
        files_to_reanalyze: to_reanalyze.clone(),
        rerun_architecture: matches!(action, Action::ArchitectureUpdate),
        rerun_tour: matches!(action, Action::ArchitectureUpdate | Action::FullUpdate),
        file_changes: changes,
    };

    if args.plan_only {
        // Persist the freshly-computed fingerprints even on the plan-only
        // path. Without this the post-commit hook plans the same set of
        // files on every invocation — once a file's hash matches the
        // stored print, the next plan must show "no changes" and let the
        // hook short-circuit. The plan itself is still emitted to
        // stdout so downstream tooling sees the proposed action.
        storage.write_fingerprints(&new_prints).await?;
        storage.save(&layout).await?;
        let json = serde_json::to_string_pretty(&plan)?;
        println!("{json}");
        return Ok(());
    }

    // Apply the partial update directly when the action allows it. The
    // LLM-driven re-analysis (which the hook would normally trigger
    // through the file-analyzer agent) is out of scope for this binary —
    // we replace the file/function/class slice with a fresh skeleton so
    // the persisted graph at least matches the tree.
    if matches!(plan.action, Action::Skip) {
        // Refresh fingerprints so future runs see the no-op.
        storage.write_fingerprints(&new_prints).await?;
        storage.save(&layout).await?;
        cleanup_scratch_dirs(&layout);
        println!("no source files changed; fingerprints refreshed");
        return Ok(());
    }

    if matches!(plan.action, Action::FullUpdate) {
        anyhow::bail!(
            "full-update threshold crossed ({total_changes} files); rerun with `analyze --full`"
        );
    }

    // Drop the stored embeddings of every node whose underlying text is
    // about to change. The next `understandable embed` call repopulates
    // only the rows that disappeared, so we don't pay for re-embedding
    // unchanged code.
    let stale_node_ids: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| match &n.file_path {
            Some(p) => to_reanalyze.iter().any(|r| r == p) || deleted.iter().any(|d| d == p),
            None => false,
        })
        .map(|n| n.id.clone())
        .collect();
    if !stale_node_ids.is_empty() {
        storage.forget_embeddings(&stale_node_ids).await?;
        // Cache hits are keyed off `(node_id, prompt_hash, file_hash)`
        // — the hash mismatch alone would already short-circuit a stale
        // hit, but dropping the entry on deletion keeps the archive
        // from accumulating tombstones for vanished files.
        storage.forget_llm_outputs(&stale_node_ids).await?;
    }

    apply_partial_update(&mut graph, &to_reanalyze, &deleted, project_path);
    let now = iso8601_now();
    graph.project.git_commit_hash = git_hash.to_string();
    graph.project.analyzed_at = now;
    if !project_name.is_empty() {
        graph.project.name = project_name.to_string();
    }
    if let Some(desc) = settings.project.description.as_deref() {
        graph.project.description = desc.to_string();
    }
    graph.layers = detect_layers(&graph);
    if matches!(plan.action, Action::ArchitectureUpdate) {
        graph.tour = generate_heuristic_tour(&graph);
    }

    storage.save_graph(&graph).await?;
    storage.write_fingerprints(&new_prints).await?;
    storage.save(&layout).await?;
    cleanup_scratch_dirs(&layout);

    println!(
        "incremental analyze: action={:?} reanalysed={} deleted={} → graph now {} nodes, {} edges",
        plan.action,
        to_reanalyze.len(),
        deleted.len(),
        graph.nodes.len(),
        graph.edges.len(),
    );

    if args.review {
        // Richer per-file diff summary for the post-commit hook.
        let added = to_reanalyze
            .iter()
            .filter(|p| !stored_by_path.contains_key(*p))
            .count();
        println!(
            "review: {} reanalysed, {} deleted, {} added (action={:?})",
            to_reanalyze.len(),
            deleted.len(),
            added,
            plan.action,
        );
    }

    handle_auto_update_flag(args.auto_update, args.no_auto_update, project_path);

    if settings.embeddings.embed_on_analyze {
        if let Err(e) = run_auto_embed(project_path).await {
            tracing::warn!(error = %e, "embed_on_analyze step failed; run `understandable embed` manually");
        }
    }
    Ok(())
}

/// Outcome of one per-file LLM task. The text is the parsed reply
/// (cached or live); `usage` is `None` on cache hits since no API call
/// was made.
struct FileOutcome {
    rel: String,
    meta: FileMeta,
    usage: Option<ua_llm::TokenUsage>,
    cached: bool,
}

/// Run Anthropic over each file (capped) to fill in summary/tags/
/// complexity. Returns a `path → FileMeta` map. Errors on a single
/// file are logged and skipped — a partial enrichment is still useful.
///
/// The loop spawns up to `concurrency` calls in parallel via
/// [`tokio::task::JoinSet`], gated by a [`tokio::sync::Semaphore`] so
/// the in-flight count never exceeds the cap. Each task consults the
/// per-file response cache first (`Storage::llm_output_for`); a hit
/// short-circuits the API call entirely.
///
/// Token usage is aggregated across every successful call and printed
/// as a single line before the function returns. Cache hits and live
/// calls are counted separately so `analyzed N files (cache hits: H/N
/// …)` is accurate even when every file came from the cache.
#[allow(clippy::too_many_arguments)]
async fn run_llm_summaries(
    project_name: &str,
    project_path: &Path,
    files: &[PathBuf],
    model: Option<String>,
    cap: usize,
    concurrency: usize,
    temperature: f32,
    storage: Arc<Storage>,
) -> anyhow::Result<std::collections::HashMap<String, FileMeta>> {
    let client = Arc::new(AnthropicClient::new(None)?);
    let registry = Arc::new(LanguageRegistry::default_registry());
    // `cap` already bounds total work; `concurrency` bounds in-flight
    // calls. Both saturate at 1 to defend against a 0 in either knob.
    let concurrency = concurrency.max(1);
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut joinset: tokio::task::JoinSet<anyhow::Result<Option<FileOutcome>>> =
        tokio::task::JoinSet::new();

    let project_name = project_name.to_string();
    let project_path: PathBuf = project_path.to_path_buf();

    for file in files.iter().take(cap) {
        let permit = sem.clone().acquire_owned().await.expect("semaphore not closed");
        let storage = storage.clone();
        let client = client.clone();
        let registry = registry.clone();
        let model = model.clone();
        let project_name = project_name.clone();
        let project_path = project_path.clone();
        let file = file.clone();
        joinset.spawn(async move {
            // The permit lives until the task drops it.
            let _permit = permit;
            process_one_file(
                &project_name,
                &project_path,
                &file,
                &registry,
                &client,
                model.as_deref(),
                temperature,
                &storage,
            )
            .await
        });
    }

    let mut metas: std::collections::HashMap<String, FileMeta> =
        std::collections::HashMap::new();
    let mut totals = TokenTotals::default();
    let mut hits = 0usize;
    let mut misses = 0usize;

    while let Some(joined) = joinset.join_next().await {
        match joined {
            Ok(Ok(Some(outcome))) => {
                if outcome.cached {
                    hits += 1;
                } else {
                    misses += 1;
                }
                if let Some(u) = outcome.usage.as_ref() {
                    totals.add(u);
                }
                metas.insert(outcome.rel, outcome.meta);
            }
            Ok(Ok(None)) => {
                // Skipped (binary / oversized / read failed). Don't
                // count it against the user — the file just doesn't
                // contribute to enrichment.
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "file task failed");
            }
            Err(e) => {
                tracing::warn!(error = %e, "file task panicked");
            }
        }
    }

    if files.len() > cap {
        tracing::info!(
            "LLM file cap ({cap}) reached; remaining files keep the empty stub meta"
        );
    }

    let total = hits + misses;
    let model_name = model
        .as_deref()
        .unwrap_or(ua_llm::ANTHROPIC_DEFAULT)
        .to_string();
    if totals.is_zero() {
        // Either nothing ran, or every file came from the cache.
        if total > 0 {
            println!("analyzed {total} files (cached: {hits}/{total})");
        }
    } else {
        let usd = totals.estimate_usd(&model_name);
        println!(
            "analyzed {total} files (cache hits: {hits}/{total}, input={} output={} cache_read={} tokens, ≈${:.4})",
            totals.input, totals.output, totals.cache_read, usd
        );
    }
    Ok(metas)
}

/// Per-file worker — extracted from the spawn closure so the borrow /
/// lifetime story stays manageable. Returns:
///   * `Ok(Some(outcome))` on success (cache hit *or* live call);
///   * `Ok(None)` when the file was skipped (binary, oversized, or
///     unreadable) — caller treats it as a no-op;
///   * `Err(_)` when the cache layer itself fails. LLM call / parse
///     failures are logged and surface as `Ok(None)` so a single bad
///     file doesn't blow up the run.
#[allow(clippy::too_many_arguments)]
async fn process_one_file(
    project_name: &str,
    project_path: &Path,
    file: &Path,
    registry: &LanguageRegistry,
    client: &AnthropicClient,
    model: Option<&str>,
    temperature: f32,
    storage: &Storage,
) -> anyhow::Result<Option<FileOutcome>> {
    let rel = relative(file, project_path);
    let language = registry
        .for_path(file)
        .map(|c| c.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let Ok(content) = tokio::fs::read_to_string(file).await else {
        return Ok(None);
    };
    if content.len() > 60_000 {
        return Ok(None);
    }

    let req = FileSummaryRequest {
        project_name,
        language: &language,
        path: &rel,
        content: &content,
    };
    let (system, user) = file_summary_prompts(&req);

    // Cache fingerprint: file body + the exact prompt template the
    // LLM would see. Changing the prompt invalidates every entry; so
    // does editing the file. The `node_id` matches the convention used
    // by `apply_partial_update` (`file_node_id`) so cleanup paths can
    // share the same key.
    let file_hash = blake3_string(content.as_bytes());
    let prompt_hash = compute_prompt_hash(&system, &user);
    let node_id = file_node_id(&rel);

    if let Some(cached) = storage
        .llm_output_for(&node_id, &prompt_hash, &file_hash)
        .await?
    {
        match parse_file_summary(&cached) {
            Ok(parsed) => {
                let complexity = match parsed.complexity.as_str() {
                    "simple" => Complexity::Simple,
                    "complex" => Complexity::Complex,
                    _ => Complexity::Moderate,
                };
                return Ok(Some(FileOutcome {
                    rel,
                    meta: FileMeta {
                        summary: parsed.summary,
                        tags: parsed.tags,
                        complexity,
                    },
                    usage: None,
                    cached: true,
                }));
            }
            Err(e) => {
                // Cached payload is unparseable — drop through to a
                // fresh call. Don't propagate; the cache will be
                // overwritten with a healthy response below.
                tracing::warn!(
                    path = %file.display(),
                    error = %e,
                    "cached llm reply failed to parse — re-running"
                );
            }
        }
    }

    let mut chat = CompleteRequest::user(user)
        .with_system(system)
        .with_max_tokens(512)
        // Temperature comes from `llm.temperature` (default 0.2); a
        // user can dial it up to encourage more creative summaries
        // without recompiling. Negative values would be a config
        // error in any provider, but `with_temperature` is a passthru
        // so we let the API surface that error rather than guessing.
        .with_temperature(temperature)
        // The system prompt is the same across every file in this
        // run — caching it cuts the per-call input bill to 10% after
        // the first hit. `with_system_cache` is idempotent and
        // safe even on providers that don't support it (the wire
        // format degrades to a plain string).
        .with_system_cache();
    if let Some(m) = model {
        chat = chat.with_model(m.to_string());
    }

    let result = match client.complete_with_usage(chat).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(path = %file.display(), error = %e, "llm call failed");
            return Ok(None);
        }
    };

    let parsed = match parse_file_summary(&result.text) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(path = %file.display(), error = %e, "could not parse llm reply");
            return Ok(None);
        }
    };

    // Persist the raw response — caller decides parsing on the next
    // run, which means a future change to `parse_file_summary` doesn't
    // require a cache flush.
    if let Err(e) = storage
        .cache_llm_output(&node_id, &prompt_hash, &file_hash, &result.text)
        .await
    {
        // Cache write failures are non-fatal — we still have a usable
        // reply for this run.
        tracing::warn!(error = %e, path = %rel, "could not cache llm output");
    }

    let complexity = match parsed.complexity.as_str() {
        "simple" => Complexity::Simple,
        "complex" => Complexity::Complex,
        _ => Complexity::Moderate,
    };
    Ok(Some(FileOutcome {
        rel,
        meta: FileMeta {
            summary: parsed.summary,
            tags: parsed.tags,
            complexity,
        },
        usage: Some(result.usage),
        cached: false,
    }))
}

fn relative(path: &Path, project: &Path) -> String {
    match path.strip_prefix(project) {
        Ok(r) => r.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Remove `<storage>/intermediate/` and `<storage>/tmp/` after a
/// successful analyze. The agent pipeline writes per-file scratch JSON
/// in `intermediate/` and the storage layer occasionally leaves stray
/// `tmp` artefacts on crash recovery; both are safe to drop once the
/// graph has been persisted. Any `NotFound` is fine — we just want a
/// clean state on disk before the next run.
fn cleanup_scratch_dirs(layout: &ProjectLayout) {
    for dir in [layout.intermediate_dir(), layout.root.join("tmp")] {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                tracing::debug!(
                    target: "ua_cli::analyze",
                    path = %dir.display(),
                    "removed scratch directory"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    target: "ua_cli::analyze",
                    path = %dir.display(),
                    "scratch directory absent; nothing to clean"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "ua_cli::analyze",
                    path = %dir.display(),
                    error = %e,
                    "could not remove scratch directory"
                );
            }
        }
    }
}

/// Try to fetch the *old* content of a tracked file from `HEAD`. Used
/// by the change classifier to compare working-tree bytes against the
/// last committed version. Returns `None` if git isn't available, the
/// file isn't tracked, or the show command failed for any reason —
/// the caller falls back to `Structural` in that case.
fn read_old_content(project: &Path, rel_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("show")
        .arg(format!("HEAD:{rel_path}"))
        .current_dir(project)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn compute_fingerprints(project: &Path, files: &[PathBuf]) -> Vec<Fingerprint> {
    let lang_registry = LanguageRegistry::default_registry();
    let plugin_registry = default_registry();
    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let rel = relative(path, project);
        let Ok(hash) = blake3_file(path) else {
            continue;
        };
        // Read the file once for the structural hash. We could keep the
        // existing streamed `blake3_file` for the byte hash and read
        // only when language is recognised — but the post-walk file set
        // is dominated by sources we'd parse anyway, and a single
        // read_to_string is cheaper than a second IO pass downstream.
        let structural_hash = compute_structural_hash(
            &lang_registry,
            &plugin_registry,
            path,
            &rel,
            None,
        );
        out.push(Fingerprint {
            path: rel,
            hash,
            modified_at: modtime_secs(path),
            structural_hash,
        });
    }
    out
}

/// Compute the structural hash for `path`. If `content` is supplied we
/// hash that directly (avoids a second read when the caller has the
/// file body in memory already); otherwise we read it from disk. Returns
/// `None` if the language has no plugin, the file isn't readable, or
/// the parser failed — every caller stores the optional value as-is so
/// the change classifier can fall back to its regex path.
fn compute_structural_hash(
    lang_registry: &LanguageRegistry,
    plugin_registry: &PluginRegistry,
    path: &Path,
    rel: &str,
    content: Option<&str>,
) -> Option<String> {
    let lang = lang_registry.for_path(path).map(|c| c.id.clone())?;
    if !plugin_registry.supports(&lang) {
        return None;
    }
    match content {
        Some(c) => plugin_registry.structural_hash_of(&lang, rel, c),
        None => {
            let body = std::fs::read_to_string(path).ok()?;
            plugin_registry.structural_hash_of(&lang, rel, &body)
        }
    }
}

fn any_directory_change(
    deleted: &[String],
    reanalysed: &[String],
    graph: &KnowledgeGraph,
) -> bool {
    let known_dirs: BTreeSet<String> = graph
        .nodes
        .iter()
        .filter_map(|n| n.file_path.as_deref())
        .map(|p| p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default())
        .collect();
    let touched: BTreeSet<String> = deleted
        .iter()
        .chain(reanalysed.iter())
        .map(|p| p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default())
        .collect();
    touched.iter().any(|d| !known_dirs.contains(d))
}

fn apply_partial_update(
    graph: &mut KnowledgeGraph,
    reanalysed: &[String],
    deleted: &[String],
    project: &Path,
) {
    let mut affected_paths: BTreeSet<&str> = BTreeSet::new();
    for p in reanalysed {
        affected_paths.insert(p.as_str());
    }
    for p in deleted {
        affected_paths.insert(p.as_str());
    }

    // Stash the existing nodes for every reanalysed path *before* we
    // drop them so we can preserve summary/tags/complexity that already
    // landed (from a prior `--with-llm` run, manual edits, etc).
    // Without this, every incremental analyze blew the LLM-enriched
    // metadata back to defaults, forcing a full re-run on the next call.
    let preserved: BTreeMap<String, GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.file_path
                .as_deref()
                .map(|p| reanalysed.iter().any(|r| r == p))
                .unwrap_or(false)
                && n.id == file_node_id(n.file_path.as_deref().unwrap_or(""))
        })
        .map(|n| (n.id.clone(), n.clone()))
        .collect();

    // Drop nodes whose `file_path` is in the affected set.
    graph.nodes.retain(|n| {
        !n.file_path
            .as_deref()
            .map(|p| affected_paths.contains(p))
            .unwrap_or(false)
    });

    // Drop dangling edges.
    let known_ids: BTreeSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    graph
        .edges
        .retain(|e| known_ids.contains(&e.source) && known_ids.contains(&e.target));

    // Re-add a stub file node for every still-existing path in
    // `reanalysed`. The full analyzer pipeline (LLM + tree-sitter
    // extraction) lands once `ua-llm` is in — for now the stub keeps
    // the graph's file inventory in sync with the filesystem. When the
    // node previously existed we copy the LLM-derived metadata over so
    // the partial update doesn't silently regress quality.
    for path in reanalysed {
        let abs = project.join(path);
        if !abs.exists() {
            continue;
        }
        let id = file_node_id(path);
        if graph.nodes.iter().any(|n| n.id == id) {
            continue;
        }
        let basename = abs
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();

        let prior = preserved.get(&id);
        // For renamed files (path appears in `reanalysed` but no prior
        // node existed under that id) seed the summary with basename +
        // path. This guarantees two renamed files don't end up with
        // identical empty embedding text — `node_text` would then
        // produce the same `name :: summary :: tags` triple and the
        // bulk embedder would treat them as duplicates.
        let (summary, tags, complexity, language_notes, domain_meta, knowledge_meta) =
            match prior {
                Some(p) => (
                    p.summary.clone(),
                    p.tags.clone(),
                    p.complexity,
                    p.language_notes.clone(),
                    p.domain_meta.clone(),
                    p.knowledge_meta.clone(),
                ),
                None => {
                    tracing::info!(
                        target: "ua_cli::analyze",
                        path = %path,
                        "needs-llm: reanalysed file has no prior node — seeded stub summary, run `analyze --with-llm` to enrich"
                    );
                    (
                        format!("{basename} ({path})"),
                        Vec::new(),
                        Complexity::Moderate,
                        None,
                        None,
                        None,
                    )
                }
            };

        graph.nodes.push(GraphNode {
            id,
            node_type: NodeType::File,
            name: basename,
            file_path: Some(path.clone()),
            line_range: None,
            summary,
            tags,
            complexity,
            language_notes,
            domain_meta,
            knowledge_meta,
        });
    }
}

fn file_node_id(path: &str) -> String {
    format!("file:{path}")
}

/// Resolve the effective project name with the documented precedence:
/// `--name` (CLI) → `project.name` (settings) → directory basename.
fn resolve_project_name(
    cli_name: Option<&str>,
    settings: &ProjectSettings,
    project_path: &Path,
) -> String {
    if let Some(n) = cli_name {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(n) = settings.project.name.as_deref() {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    project_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string()
}

/// `--llm-max-files` semantics: 0 means "use settings", non-zero is
/// taken as-is. The settings fallback is itself defaulted to 50, so the
/// final number is never zero.
fn resolve_llm_max_files(cli: usize, settings: &ProjectSettings) -> usize {
    if cli > 0 {
        return cli;
    }
    let from_settings = settings.llm.max_files;
    if from_settings == 0 {
        50
    } else {
        from_settings
    }
}

/// Length-prefixed transcript hash. The previous implementation joined
/// `system` and `user` with `'|'` which would collide for any two
/// (system, user) pairs that round-tripped through the same boundary
/// (e.g. `system="a", user="b|c"` and `system="a|b", user="c"` shared a
/// digest). Encoding lengths up front makes the boundary unambiguous.
fn compute_prompt_hash(system: &str, user: &str) -> String {
    let mut buf: Vec<u8> = Vec::with_capacity(
        b"sys".len() + 8 + system.len() + b"usr".len() + 8 + user.len(),
    );
    buf.extend_from_slice(b"sys");
    buf.extend_from_slice(&(system.len() as u64).to_le_bytes());
    buf.extend_from_slice(system.as_bytes());
    buf.extend_from_slice(b"usr");
    buf.extend_from_slice(&(user.len() as u64).to_le_bytes());
    buf.extend_from_slice(user.as_bytes());
    blake3_string(&buf)
}

/// Process the `--auto-update` / `--no-auto-update` pair. The signal
/// file lives at `<project>/.understandable/auto-update.signal` and is
/// watched by the post-commit hook + IDE plugins to know the graph was
/// just refreshed. `--no-auto-update` removes any stale signal so the
/// next watcher tick doesn't fire on a previous run's leftovers.
fn handle_auto_update_flag(auto: bool, no_auto: bool, project_path: &Path) {
    let layout = ProjectLayout::for_project(project_path);
    let signal = layout.root.join("auto-update.signal");
    if auto {
        if let Some(parent) = signal.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(error = %e, "could not ensure storage dir for auto-update signal");
                return;
            }
        }
        let payload = format!("{{\"timestamp\":\"{}\"}}\n", iso8601_now());
        if let Err(e) = std::fs::write(&signal, payload) {
            tracing::warn!(error = %e, path = %signal.display(), "could not write auto-update signal");
        } else {
            tracing::info!(target: "ua_cli::analyze", path = %signal.display(), "auto-update signal written");
        }
    } else if no_auto {
        match std::fs::remove_file(&signal) {
            Ok(()) => {
                tracing::info!(target: "ua_cli::analyze", "auto-update signal cleared");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(error = %e, path = %signal.display(), "could not remove auto-update signal");
            }
        }
    }
}

/// JSON document emitted by `--scan-only`. Mirrors the TS
/// `project-scanner` agent's contract so downstream consumers (the
/// markdown agents, IDE plugins) can speak the same wire shape.
#[derive(Debug, Serialize)]
struct ScanResult {
    name: String,
    languages: Vec<String>,
    frameworks: Vec<String>,
    files: Vec<ScanFile>,
    #[serde(rename = "totalFiles")]
    total_files: usize,
    #[serde(rename = "filteredByIgnore")]
    filtered_by_ignore: usize,
    #[serde(rename = "estimatedComplexity")]
    estimated_complexity: String,
    #[serde(rename = "importMap")]
    import_map: BTreeMap<String, Vec<String>>,
    #[serde(rename = "rawDescription", skip_serializing_if = "Option::is_none")]
    raw_description: Option<String>,
    #[serde(rename = "readmeHead", skip_serializing_if = "Option::is_none")]
    readme_head: Option<String>,
    #[serde(rename = "scriptCompleted")]
    script_completed: bool,
}

#[derive(Debug, Serialize)]
struct ScanFile {
    path: String,
    #[serde(rename = "fileCategory")]
    file_category: &'static str,
}

/// Heuristic file-category bucket used by `ScanFile.fileCategory`.
fn classify_file_category(rel: &str) -> &'static str {
    let lower = rel.to_ascii_lowercase();
    let basename = lower.rsplit('/').next().unwrap_or(lower.as_str());
    // Doc files first — `.md` inside a `tests/` dir is still a doc.
    if lower.ends_with(".md") || lower.ends_with(".rst") || lower.ends_with(".txt") {
        return "doc";
    }
    if lower.ends_with(".json") || lower.ends_with(".yaml") || lower.ends_with(".yml")
        || lower.ends_with(".toml")
    {
        // Config-shaped names live in this bucket too. Distinguish
        // package manifests (config) from data dumps (data) by name.
        const CONFIG_BASENAMES: &[&str] = &[
            "package.json",
            "tsconfig.json",
            "cargo.toml",
            "pyproject.toml",
            "pipfile",
            "go.mod",
            "go.sum",
            "gemfile",
            "rakefile",
            "dockerfile",
            "makefile",
            ".eslintrc",
            ".prettierrc",
            "understandable.yaml",
            "understandable.yml",
        ];
        if CONFIG_BASENAMES.contains(&basename) {
            return "config";
        }
        return "data";
    }
    if lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("/__tests__/")
        || basename.contains("test")
        || basename.contains("spec")
    {
        return "test";
    }
    const CONFIG_NAMES: &[&str] = &[
        "dockerfile",
        "makefile",
        "rakefile",
        "gemfile",
        ".gitignore",
        ".dockerignore",
        ".editorconfig",
        "understandignore",
    ];
    if CONFIG_NAMES.contains(&basename) {
        return "config";
    }
    "source"
}

/// Three-tier complexity estimate by total LOC. Matches the rough
/// buckets the TS agent emits — `small` < 5k, `medium` < 50k, `large`
/// otherwise — so downstream templating can branch the same way.
fn estimate_complexity(total_lines: usize) -> &'static str {
    if total_lines < 5_000 {
        "small"
    } else if total_lines < 50_000 {
        "medium"
    } else {
        "large"
    }
}

/// `--scan-only` entry point. Walks the project once with the configured
/// ignore filter, classifies every file, sniffs frameworks via the
/// shared registry, and prints the assembled JSON. No persistence: this
/// path is meant to be cheap and idempotent.
async fn run_scan_only(
    project_path: &Path,
    project_name: &str,
    settings: &ProjectSettings,
) -> anyhow::Result<()> {
    let filter = IgnoreFilter {
        extra_ignore_paths: settings.ignore.paths.clone(),
        ..IgnoreFilter::default()
    };
    let files: Vec<PathBuf> = walk_project(project_path, &filter).collect();

    // To estimate `filteredByIgnore` we re-walk with the user's ignore
    // paths cleared. The two walks share the same `.gitignore` /
    // `.understandignore` rules, so the delta is exactly the count of
    // files dropped by `ignore.paths`.
    let baseline_filter = IgnoreFilter::default();
    let baseline_total: usize = walk_project(project_path, &baseline_filter).count();
    let filtered_by_ignore = baseline_total.saturating_sub(files.len());

    let lang_registry = LanguageRegistry::default_registry();
    let fw_registry = FrameworkRegistry::default_registry();

    let mut languages: BTreeSet<String> = BTreeSet::new();
    let mut scan_files: Vec<ScanFile> = Vec::with_capacity(files.len());
    let mut total_lines: usize = 0;
    let mut manifests: Vec<(PathBuf, String)> = Vec::new();
    let manifest_basenames: BTreeSet<String> = fw_registry
        .all()
        .iter()
        .flat_map(|fw| fw.manifest_files.iter().cloned())
        .map(|s| s.to_ascii_lowercase())
        .collect();

    for file in &files {
        let rel = relative(file, project_path);
        if let Some(cfg) = lang_registry.for_path(file) {
            languages.insert(cfg.id.clone());
        }
        // Cheap line-count probe — a file's length in newlines is a
        // good enough proxy for "complexity" at this layer. Skip
        // unreadable files instead of failing the whole scan.
        if let Ok(content) = std::fs::read_to_string(file) {
            total_lines += content.lines().count();
            // Stash manifest contents for framework detection. Limit
            // the scan to files whose basename is a known manifest so
            // we don't pull every JSON in the tree into memory.
            if let Some(name) = file
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
            {
                if manifest_basenames.contains(&name) {
                    manifests.push((file.clone(), content));
                }
            }
        }
        scan_files.push(ScanFile {
            path: rel,
            file_category: classify_file_category(file.to_string_lossy().as_ref()),
        });
    }

    let manifest_refs: Vec<(&Path, &str)> = manifests
        .iter()
        .map(|(p, c)| (p.as_path(), c.as_str()))
        .collect();
    // No import data at this layer — the analyzer hasn't run yet.
    // Manifest detection alone covers the common case (package.json
    // → React, Cargo.toml → axum, etc.).
    let frameworks = ua_extract::detect_frameworks(&fw_registry, &manifest_refs, &[]);

    let readme_head = read_readme_head(project_path);
    let raw_description = settings.project.description.clone().filter(|s| !s.is_empty());

    let result = ScanResult {
        name: project_name.to_string(),
        languages: languages.into_iter().collect(),
        frameworks,
        total_files: scan_files.len(),
        filtered_by_ignore,
        estimated_complexity: estimate_complexity(total_lines).to_string(),
        files: scan_files,
        // The CLI scanner doesn't compute import resolutions —
        // that's the analyzer's job. Emit an empty map so the schema
        // shape is stable; downstream consumers can skip the field.
        import_map: BTreeMap::new(),
        raw_description,
        readme_head,
        script_completed: true,
    };

    let json = serde_json::to_string_pretty(&result)?;
    println!("{json}");
    Ok(())
}

/// Read up to 1 KiB of `README.md` (case-insensitive) from the project
/// root. Returns `None` if no README exists or it can't be read.
fn read_readme_head(project_path: &Path) -> Option<String> {
    const CANDIDATES: &[&str] = &["README.md", "readme.md", "Readme.md"];
    for name in CANDIDATES {
        let path = project_path.join(name);
        if let Ok(bytes) = std::fs::read(&path) {
            let take = bytes.len().min(1024);
            // Trim to a UTF-8 boundary so multi-byte chars survive
            // the cut. `from_utf8_lossy` would silently lose data.
            let mut end = take;
            while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
                end -= 1;
            }
            if let Ok(s) = std::str::from_utf8(&bytes[..end]) {
                return Some(s.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ua_core::meta::ProjectMeta;
    use ua_core::{Complexity, GraphNode, KnowledgeGraph, NodeType};

    fn empty_graph() -> KnowledgeGraph {
        KnowledgeGraph::new(ProjectMeta {
            name: "t".into(),
            languages: vec![],
            frameworks: vec![],
            description: String::new(),
            analyzed_at: "1970-01-01T00:00:00Z".into(),
            git_commit_hash: String::new(),
        })
    }

    /// Cheap RAII tempdir without pulling in the `tempfile` crate as a
    /// dev-dep on `ua-cli`. Tests deposit files inside the OS temp dir
    /// under a unique subfolder and clean up on drop.
    struct ScratchDir {
        path: std::path::PathBuf,
    }

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "ua-cli-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn enriched_node(path: &str) -> GraphNode {
        GraphNode {
            id: file_node_id(path),
            node_type: NodeType::File,
            name: std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
                .to_string(),
            file_path: Some(path.to_string()),
            line_range: None,
            summary: "hand-crafted summary".into(),
            tags: vec!["service".into(), "api".into()],
            complexity: Complexity::Complex,
            language_notes: Some("uses async/await".into()),
            domain_meta: None,
            knowledge_meta: None,
        }
    }

    #[test]
    fn reanalysed_path_preserves_prior_metadata() {
        // Stand up a temp project so the file referenced in `reanalysed`
        // actually exists on disk — `apply_partial_update` skips the
        // stub re-add otherwise.
        let tmp = ScratchDir::new("preserve");
        let rel = "src/keep.rs";
        let abs = tmp.path().join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, "fn main() {}\n").unwrap();

        let mut graph = empty_graph();
        graph.nodes.push(enriched_node(rel));

        apply_partial_update(&mut graph, &[rel.to_string()], &[], tmp.path());

        let node = graph
            .nodes
            .iter()
            .find(|n| n.id == file_node_id(rel))
            .expect("re-added node");
        assert_eq!(node.summary, "hand-crafted summary");
        assert_eq!(node.tags, vec!["service".to_string(), "api".to_string()]);
        assert_eq!(node.complexity, Complexity::Complex);
        assert_eq!(node.language_notes.as_deref(), Some("uses async/await"));
    }

    #[test]
    fn renamed_path_seeds_unique_stub_summary() {
        // The "rename" pattern: the *new* path appears in `reanalysed`
        // but the prior node was under a different id. The new node
        // should get a stub summary that includes basename + path so
        // its embedding text is unique.
        let tmp = ScratchDir::new("rename");
        let new_rel = "src/renamed_target.rs";
        let abs = tmp.path().join(new_rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, "fn main() {}\n").unwrap();

        let mut graph = empty_graph();
        // Pre-existing node under a *different* path — the rename
        // looks like (delete old) + (add new) from the planner's view.
        graph.nodes.push(enriched_node("src/old_name.rs"));

        apply_partial_update(
            &mut graph,
            &[new_rel.to_string()],
            &["src/old_name.rs".to_string()],
            tmp.path(),
        );

        let node = graph
            .nodes
            .iter()
            .find(|n| n.id == file_node_id(new_rel))
            .expect("new node added");

        // Stub summary includes both basename + path so two renames in
        // the same incremental run can't collide on identical
        // embedding text.
        assert!(
            node.summary.contains("renamed_target.rs"),
            "summary should include basename, got: {:?}",
            node.summary
        );
        assert!(
            node.summary.contains(new_rel),
            "summary should include the full path, got: {:?}",
            node.summary
        );
        // The old-name node was scheduled for deletion ⇒ removed.
        assert!(
            graph
                .nodes
                .iter()
                .all(|n| n.id != file_node_id("src/old_name.rs")),
            "deleted-side node should be dropped"
        );
    }

    /// Regression: the old `format!("{system}|{user}")` collided on any
    /// `'|'` boundary shift between the two strings. `compute_prompt_hash`
    /// emits a length-prefixed transcript so the boundary is unambiguous
    /// and the two inputs hash to distinct digests.
    #[test]
    fn prompt_hash_separator_collision_is_avoided() {
        // Pre-fix bug: both pairs collapse to `"a|b|c"` after the join.
        let h1 = compute_prompt_hash("a", "b|c");
        let h2 = compute_prompt_hash("a|b", "c");
        assert_ne!(
            h1, h2,
            "pipe boundary should not let two distinct (system, user) pairs collide"
        );

        // Sanity: deterministic and stable across re-invocation.
        assert_eq!(h1, compute_prompt_hash("a", "b|c"));

        // Empty-prefix variant — the length tag still distinguishes
        // these because `len("") = 0` is encoded explicitly.
        let h3 = compute_prompt_hash("", "abc");
        let h4 = compute_prompt_hash("a", "bc");
        assert_ne!(h3, h4);
    }

    /// `--llm-max-files 0` is the documented sentinel for "use the
    /// settings cascade". When the YAML override is missing we land on
    /// the library default (50).
    #[test]
    fn llm_max_files_zero_uses_settings_default() {
        // Default settings → llm.max_files == 50.
        let s = ProjectSettings::recommended();
        assert_eq!(resolve_llm_max_files(0, &s), 50);

        // YAML override wins when CLI is 0.
        let mut s2 = ProjectSettings::recommended();
        s2.llm.max_files = 17;
        assert_eq!(resolve_llm_max_files(0, &s2), 17);

        // Explicit CLI value beats both.
        assert_eq!(resolve_llm_max_files(7, &s2), 7);

        // Pathological YAML (max_files = 0) falls back to the
        // library default rather than disabling the LLM loop.
        let mut s3 = ProjectSettings::recommended();
        s3.llm.max_files = 0;
        assert_eq!(resolve_llm_max_files(0, &s3), 50);
    }

    /// `incremental.full_threshold` and `incremental.big_graph_threshold`
    /// drive the change-plan action selector. We don't have a public
    /// helper here, so we exercise the same arithmetic the
    /// `run_incremental` body uses and assert it matches the YAML
    /// override.
    #[test]
    fn incremental_threshold_reads_from_settings() {
        let mut s = ProjectSettings::recommended();
        s.incremental.full_threshold = 5;
        s.incremental.big_graph_threshold = 10;

        // Re-create the `run_incremental` decision: at 6 changes the
        // FullUpdate branch must fire because `total_changes >
        // full_threshold` is true.
        let total_changes = 6usize;
        let nodes = 4usize; // graph below big_graph_threshold
        let big_graph = nodes >= s.incremental.big_graph_threshold;
        let full_triggered = total_changes > s.incremental.full_threshold
            || (big_graph && total_changes * 2 > nodes);
        assert!(
            full_triggered,
            "6 changes against full_threshold=5 should recommend a full rebuild"
        );

        // Bump the threshold past the change count → no full rebuild.
        let mut s2 = ProjectSettings::recommended();
        s2.incremental.full_threshold = 100;
        s2.incremental.big_graph_threshold = 100;
        let big_graph2 = nodes >= s2.incremental.big_graph_threshold;
        let full_triggered2 = total_changes > s2.incremental.full_threshold
            || (big_graph2 && total_changes * 2 > nodes);
        assert!(
            !full_triggered2,
            "6 changes against full_threshold=100 should stay incremental"
        );
    }

    #[test]
    fn resolve_project_name_precedence_holds() {
        let tmp = ScratchDir::new("name-precedence");
        let mut s = ProjectSettings::recommended();

        // No CLI, no settings → directory basename wins.
        let bare = resolve_project_name(None, &s, tmp.path());
        assert!(
            bare.starts_with("ua-cli-test-name-precedence"),
            "expected basename, got {bare}"
        );

        // Settings populates → settings wins over basename.
        s.project.name = Some("from-settings".into());
        assert_eq!(
            resolve_project_name(None, &s, tmp.path()),
            "from-settings"
        );

        // CLI populates → CLI wins over both.
        assert_eq!(
            resolve_project_name(Some("from-cli"), &s, tmp.path()),
            "from-cli"
        );

        // Empty CLI string is ignored.
        assert_eq!(
            resolve_project_name(Some("   "), &s, tmp.path()),
            "from-settings"
        );
    }

    #[test]
    fn classify_file_category_routes_known_paths() {
        assert_eq!(classify_file_category("README.md"), "doc");
        assert_eq!(classify_file_category("docs/intro.rst"), "doc");
        assert_eq!(classify_file_category("package.json"), "config");
        assert_eq!(classify_file_category("Cargo.toml"), "config");
        assert_eq!(classify_file_category("src/data/users.json"), "data");
        assert_eq!(
            classify_file_category("crates/foo/tests/integration.rs"),
            "test"
        );
        assert_eq!(classify_file_category("src/foo_test.go"), "test");
        assert_eq!(classify_file_category("src/foo.rs"), "source");
    }
}

