//! `understandable init` — scaffold (or update) a project-level
//! `understandable.yaml`.
//!
//! Every field of [`ProjectSettings`] is reachable via a flag so the
//! `understand-setup` LLM-led wizard can build any combination
//! deterministically. Apply order is `recommended() → preset →
//! individual flags`, so individual flags always win.
//!
//! Three opinionated presets cover the common deployment shapes:
//!   * `minimal`     — heuristic only, no LLM, no embeddings.
//!   * `local-full`  — Ollama embeddings + host LLM (no API keys).
//!   * `cloud-full`  — Anthropic + OpenAI (best quality).

use std::path::{Path, PathBuf};

use clap::{builder::BoolishValueParser, Args as ClapArgs, ValueEnum};
use ua_core::ProjectSettings;
use ua_llm::{ANTHROPIC_DEFAULT, OLLAMA_EMBED_DEFAULT, OPENAI_EMBED_DEFAULT};
use ua_persist::{apply_block, GitignoreOutcome, GitignorePolicy};

/// Hard cap on the size of any user-supplied `understandable.yaml` we
/// agree to parse. Defends against the billion-laughs class of YAML
/// attacks where a tiny file (a few KiB) inflates into millions of
/// nested aliases. Real configs run a few hundred bytes; 256 KiB is
/// generous headroom for anyone who still wants to push the format.
const MAX_SETTINGS_BYTES: u64 = 256 * 1024;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Overwrite an existing config file. By default `--force` *merges*
    /// CLI overrides into the existing YAML and writes a `.bak` of the
    /// previous file. Pair with `--no-merge` for a clean rewrite.
    #[arg(long)]
    pub force: bool,
    /// Choose an opinionated default set. Individual flags below still
    /// override the preset's choices.
    #[arg(long, value_enum)]
    pub preset: Option<Preset>,
    /// Print the resulting YAML to stdout instead of writing it. Useful
    /// for the LLM wizard so it can show the user the plan before
    /// committing. No filesystem side effects.
    #[arg(long)]
    pub dry_run: bool,
    /// Don't touch `.gitignore`. By default `init` appends (or
    /// rewrites in place) a managed block describing the storage
    /// directory based on `git.commit_db`.
    #[arg(long)]
    pub no_gitignore: bool,
    /// Skip merging with an existing `understandable.yaml` under
    /// `--force`. The previous file is replaced wholesale; only flags
    /// you pass on the command line (plus the preset / recommended
    /// defaults) survive.
    #[arg(long)]
    pub no_merge: bool,
    /// Skip writing `understandable.yaml.bak` when `--force` overwrites
    /// an existing file. By default a one-shot backup is left next to
    /// the new YAML so users can recover if the merge dropped comments
    /// or fields they cared about.
    #[arg(long)]
    pub no_backup: bool,

    // ---- project ------------------------------------------------------
    /// Override the auto-detected project name.
    #[arg(long = "project-name")]
    pub project_name: Option<String>,
    /// Free-form description copied into the graph metadata.
    #[arg(long = "project-description")]
    pub project_description: Option<String>,

    // ---- storage ------------------------------------------------------
    /// Storage directory (relative to project root or absolute).
    /// Default `.understandable`.
    #[arg(long = "storage-dir")]
    pub storage_dir: Option<String>,
    /// Filename stem for the canonical DB. Default `graph` →
    /// `<storage-dir>/graph.tar.zst`. Domain / knowledge graphs land at
    /// `<storage-dir>/<db-name>.{domain,knowledge}.tar.zst`.
    #[arg(long = "db-name")]
    pub db_name: Option<String>,

    // ---- embeddings ---------------------------------------------------
    /// Embedding provider. Restricted to a known enum so typos
    /// (`olama` vs `ollama`) are rejected at parse time rather than
    /// silently written into the YAML and exploding much later inside
    /// `embed`.
    #[arg(long, value_enum)]
    pub embed_provider: Option<EmbedProviderArg>,
    /// Embedding model id.
    #[arg(long)]
    pub embed_model: Option<String>,
    /// Override the embeddings endpoint base URL (openai-compat only).
    #[arg(long)]
    pub embed_endpoint: Option<String>,
    /// Texts per provider call.
    #[arg(long)]
    pub embed_batch_size: Option<usize>,
    /// Run `embed` automatically as the last step of `analyze`.
    #[arg(long, value_parser = BoolishValueParser::new())]
    pub embed_on_analyze: Option<bool>,
    /// How many embedding batches to run in parallel. Default 2.
    #[arg(long = "embed-concurrency")]
    pub embed_concurrency: Option<usize>,

    // ---- llm ----------------------------------------------------------
    /// LLM provider (`anthropic` / `host` / …).
    #[arg(long)]
    pub llm_provider: Option<String>,
    /// LLM model id.
    #[arg(long)]
    pub llm_model: Option<String>,
    /// Cap on files sent to the LLM in one analyze run.
    #[arg(long)]
    pub llm_max_files: Option<usize>,
    /// Sampling temperature for the LLM.
    #[arg(long)]
    pub llm_temperature: Option<f32>,
    /// Run `analyze --with-llm` automatically.
    #[arg(long, value_parser = BoolishValueParser::new())]
    pub llm_run_on_analyze: Option<bool>,
    /// How many files to send to the LLM in parallel during
    /// `analyze --with-llm`. Default 4.
    #[arg(long = "llm-concurrency")]
    pub llm_concurrency: Option<usize>,

    // ---- ignore -------------------------------------------------------
    /// Extra ignore prefix. Repeatable: `--ignore-path target/
    /// --ignore-path dist/` adds two entries.
    #[arg(long = "ignore-path")]
    pub ignore_paths: Vec<String>,

    // ---- incremental --------------------------------------------------
    /// Threshold above which `analyze --incremental` recommends a full
    /// rebuild (default 30).
    #[arg(long)]
    pub incremental_full_threshold: Option<usize>,
    /// Below this graph size the percentage check is skipped (default 50).
    #[arg(long)]
    pub incremental_big_graph_threshold: Option<usize>,

    // ---- dashboard ----------------------------------------------------
    /// Dashboard bind host.
    #[arg(long)]
    pub dashboard_host: Option<String>,
    /// Dashboard port.
    #[arg(long)]
    pub dashboard_port: Option<u16>,
    /// Whether `understandable dashboard` opens a browser tab.
    #[arg(long, value_parser = BoolishValueParser::new())]
    pub dashboard_auto_open: Option<bool>,

    // ---- git ----------------------------------------------------------
    /// Informational hint: should `.understandable/graph.tar.zst`
    /// be committed for 100 % reproducibility?
    #[arg(long, value_parser = BoolishValueParser::new())]
    pub git_commit_db: Option<bool>,
    /// Informational hint: keep embeddings alongside the graph file.
    #[arg(long, value_parser = BoolishValueParser::new())]
    pub git_commit_embeddings: Option<bool>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Preset {
    /// No LLM, no embeddings. Heuristic graph only.
    Minimal,
    /// Ollama embeddings + host-LLM only (no API keys needed).
    LocalFull,
    /// Anthropic LLM + OpenAI embeddings. Needs both API keys.
    CloudFull,
}

/// Whitelist of embedding providers accepted by `init`. Mirrors the
/// `EmbedProvider` enum used by `search`, but is defined locally so
/// the YAML conversion lives next to the flag definition. clap rejects
/// typos (e.g. `olama`) at parse time — the previous free-form
/// `String` allowed garbage values that only blew up much later inside
/// `embed`'s loose `parse_provider`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EmbedProviderArg {
    Openai,
    Ollama,
    Local,
}

impl EmbedProviderArg {
    fn as_yaml(self) -> &'static str {
        match self {
            EmbedProviderArg::Openai => "openai",
            EmbedProviderArg::Ollama => "ollama",
            EmbedProviderArg::Local => "local",
        }
    }
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let path = ProjectSettings::default_path(project);
    let existing_path = ProjectSettings::find(project);
    let existing_exists = existing_path.is_some();
    if existing_exists && !args.force && !args.dry_run {
        anyhow::bail!(
            "{} already exists — pass `--force` to overwrite",
            path.display()
        );
    }

    // Cap reads of `understandable.yaml` at 256 KiB. The public
    // `ProjectSettings::load` API doesn't expose a size knob, so we do
    // the gating here before parsing — a YAML bomb won't melt the CLI
    // on `init --force`. This intentionally only fires when a config
    // already exists; the freshly-written one is generated by us.
    if let Some(found) = existing_path.as_ref() {
        let meta = std::fs::metadata(found)?;
        if meta.len() > MAX_SETTINGS_BYTES {
            anyhow::bail!(
                "{} is {} bytes (cap {MAX_SETTINGS_BYTES}); refusing to parse",
                found.display(),
                meta.len()
            );
        }
    }

    // Pick the base settings. When --force is supplied and a parsable
    // file already exists, merge: deserialise the previous YAML, then
    // layer the preset and CLI overrides on top. This preserves
    // hand-typed fields the user set but never passed on the command
    // line. Comments cannot be retained (serde_yaml_ng has no
    // round-tripping), but we leave a `.bak` and warn loudly so the
    // user can diff.
    let mut comments_warning: Option<String> = None;
    let mut settings = match existing_path.as_ref() {
        Some(found) if args.force && !args.no_merge => {
            // Detect comments before parsing so we can warn the user
            // they'll be dropped — `serde_yaml_ng` has no round-trip /
            // comment-preserving mode. We deliberately use the public
            // `ProjectSettings::load` API rather than reaching for
            // `serde_yaml_ng::from_str` directly, since the YAML crate
            // is not a direct dep of `ua-cli`.
            let had_comments = std::fs::read_to_string(found)
                .map(|raw| raw.lines().any(|l| l.trim_start().starts_with('#')))
                .unwrap_or(false);
            match ProjectSettings::load(project) {
                Ok(Some(parsed)) => {
                    if had_comments {
                        comments_warning = Some(format!(
                            "init: comments in {} were dropped during the merge; review the diff and re-add anything you need.",
                            found.display()
                        ));
                    }
                    parsed
                }
                Ok(None) => ProjectSettings::recommended(),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "existing settings could not be parsed — falling back to recommended defaults"
                    );
                    ProjectSettings::recommended()
                }
            }
        }
        _ => ProjectSettings::recommended(),
    };

    if let Some(preset) = args.preset {
        apply_preset(&mut settings, preset);
    }
    apply_flags(&mut settings, &args);

    // Reject `..`-style traversal in `storage.dir`. Relative dirs must
    // resolve under `project`; absolute paths the user typed by hand
    // are accepted as-is (they're explicit consent to escape the
    // project root). Without this guard, `--storage-dir ../../etc`
    // could plant DB writes outside the repo. Under --dry-run the
    // validator must not touch the filesystem (no `create_dir_all`,
    // no `canonicalize` on the target path) — `init --dry-run` is
    // documented as preview-only.
    validate_storage_dir(project, &settings.storage.dir, args.dry_run)?;

    if args.dry_run {
        println!("{}", settings.to_yaml()?);
        return Ok(());
    }

    // Stash a `.bak` of the previous file before clobbering it. Cheap
    // safety net for `--force`: a typo in a single CLI flag now
    // doesn't leave the user racing `git restore`. Skipped when the
    // user opted out via `--no-backup` or there's nothing to back up.
    if existing_exists && !args.no_backup {
        if let Some(found) = existing_path.as_ref() {
            let bak = backup_path(found);
            if let Err(e) = std::fs::copy(found, &bak) {
                tracing::warn!(error = %e, path = %bak.display(), "failed to write settings backup");
            } else {
                println!("→ saved previous settings to {}", bak.display());
            }
        }
    }

    settings.save(&path)?;
    println!(
        "wrote {}\n→ commit it so every dev gets the same providers / thresholds.",
        path.display()
    );
    if let Some(msg) = comments_warning {
        println!("→ {msg}");
    }

    if !args.no_gitignore {
        let policy = GitignorePolicy::from_settings(&settings);
        match apply_block(project, policy, &settings.storage) {
            Ok(outcome) => {
                let verb = match outcome {
                    GitignoreOutcome::Created => "created",
                    GitignoreOutcome::Updated => "updated managed block in",
                    GitignoreOutcome::Appended => "appended managed block to",
                    GitignoreOutcome::AlreadyCurrent => "already up to date:",
                };
                let policy_label = match policy {
                    GitignorePolicy::CommitDb => {
                        "commit-DB mode (DB tracked, only intermediate/ + tmp/ ignored)"
                    }
                    GitignorePolicy::IgnoreAll => {
                        "ignore-all mode (whole storage dir kept out of git)"
                    }
                };
                println!(
                    "→ gitignore {verb} {} ({policy_label}). Pass `--no-gitignore` to skip.",
                    project.join(".gitignore").display()
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to update .gitignore — fix manually");
            }
        }
    } else {
        println!("→ skipped .gitignore update (--no-gitignore).");
    }
    Ok(())
}

/// Append `.bak` to whatever extension the existing settings file is
/// using. `understandable.yaml` → `understandable.yaml.bak`,
/// `understandable.yml` → `understandable.yml.bak`.
fn backup_path(found: &Path) -> PathBuf {
    let mut name = found
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".bak");
    found.with_file_name(name)
}

fn apply_preset(settings: &mut ProjectSettings, preset: Preset) {
    match preset {
        Preset::Minimal => {
            settings.embeddings.provider = "openai".into();
            settings.embeddings.model = None;
            settings.embeddings.embed_on_analyze = false;
            settings.llm.provider = "anthropic".into();
            settings.llm.model = None;
            settings.llm.run_on_analyze = false;
        }
        Preset::LocalFull => {
            settings.embeddings.provider = "ollama".into();
            settings.embeddings.model = Some(OLLAMA_EMBED_DEFAULT.into());
            settings.embeddings.endpoint = None;
            settings.embeddings.embed_on_analyze = true;
            settings.llm.provider = "host".into();
            settings.llm.model = None;
            settings.llm.run_on_analyze = false;
        }
        Preset::CloudFull => {
            settings.embeddings.provider = "openai".into();
            settings.embeddings.model = Some(OPENAI_EMBED_DEFAULT.into());
            settings.embeddings.embed_on_analyze = true;
            settings.llm.provider = "anthropic".into();
            settings.llm.model = Some(ANTHROPIC_DEFAULT.into());
            settings.llm.run_on_analyze = true;
        }
    }
}

/// Reject `storage.dir` values that escape the project root.
///
/// Logic: if `dir` is absolute, accept (the user typed an explicit
/// path). Otherwise normalise `<project>/<dir>` lexically to collapse
/// any `..` segments and confirm the result is still under the
/// canonicalised project root. `..` traversal therefore fails loudly
/// here rather than silently dumping DB writes outside the repo.
///
/// Under `dry_run` the validator must be free of filesystem side
/// effects: no `create_dir_all`, no path canonicalisation that
/// requires the target to exist. The `..` structural check and
/// absolute-path acceptance are still cheap and pure, so they remain.
fn validate_storage_dir(project: &Path, dir: &str, dry_run: bool) -> anyhow::Result<()> {
    let dir_path = Path::new(dir);
    if dir_path.is_absolute() {
        return Ok(());
    }
    if dir.contains("..") {
        // Cheap structural rejection: any `..` component is suspicious
        // and almost never legitimate in a `storage.dir`. Catching it
        // up front means the canonicalisation step below doesn't have
        // to chase symlinks across the filesystem.
        anyhow::bail!(
            "storage.dir `{dir}` contains `..` traversal — refuse to write outside the project root"
        );
    }
    if dry_run {
        // Preview-only: no `create_dir_all`, no canonicalisation. The
        // `..` rejection above is already enough to catch the bulk of
        // mischief in a relative path.
        return Ok(());
    }
    let project_canon = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let target = project_canon.join(dir_path);
    // Create the directory before canonicalising — `canonicalize`
    // requires the path to exist, and `init` is the natural place to
    // materialise it. (Skipped under `dry_run` above.)
    std::fs::create_dir_all(&target).ok();
    let target_canon = target.canonicalize().unwrap_or(target);
    if !target_canon.starts_with(&project_canon) {
        anyhow::bail!(
            "storage.dir `{dir}` resolves to `{}`, which is outside the project root `{}`. Refusing to write outside the repo.",
            target_canon.display(),
            project_canon.display()
        );
    }
    Ok(())
}

fn apply_flags(s: &mut ProjectSettings, a: &Args) {
    if let Some(v) = &a.project_name {
        s.project.name = Some(v.clone());
    }
    if let Some(v) = &a.project_description {
        s.project.description = Some(v.clone());
    }
    if let Some(v) = &a.storage_dir {
        s.storage.dir = v.clone();
    }
    if let Some(v) = &a.db_name {
        s.storage.db_name = v.clone();
    }

    if let Some(v) = a.embed_provider {
        s.embeddings.provider = v.as_yaml().to_string();
    }
    if let Some(v) = &a.embed_model {
        s.embeddings.model = Some(v.clone());
    }
    if let Some(v) = &a.embed_endpoint {
        s.embeddings.endpoint = Some(v.clone());
    }
    if let Some(v) = a.embed_batch_size {
        s.embeddings.batch_size = v.max(1);
    }
    if let Some(v) = a.embed_on_analyze {
        s.embeddings.embed_on_analyze = v;
    }
    if let Some(v) = a.embed_concurrency {
        s.embeddings.concurrency = v.max(1);
    }

    if let Some(v) = &a.llm_provider {
        s.llm.provider = v.clone();
    }
    if let Some(v) = &a.llm_model {
        s.llm.model = Some(v.clone());
    }
    if let Some(v) = a.llm_max_files {
        s.llm.max_files = v;
    }
    if let Some(v) = a.llm_temperature {
        s.llm.temperature = v;
    }
    if let Some(v) = a.llm_run_on_analyze {
        s.llm.run_on_analyze = v;
    }
    if let Some(v) = a.llm_concurrency {
        s.llm.concurrency = v.max(1);
    }

    if !a.ignore_paths.is_empty() {
        s.ignore.paths = a.ignore_paths.clone();
    }

    if let Some(v) = a.incremental_full_threshold {
        s.incremental.full_threshold = v;
    }
    if let Some(v) = a.incremental_big_graph_threshold {
        s.incremental.big_graph_threshold = v;
    }

    if let Some(v) = &a.dashboard_host {
        s.dashboard.host = v.clone();
    }
    if let Some(v) = a.dashboard_port {
        s.dashboard.port = v;
    }
    if let Some(v) = a.dashboard_auto_open {
        s.dashboard.auto_open = v;
    }

    if let Some(v) = a.git_commit_db {
        s.git.commit_db = v;
    }
    if let Some(v) = a.git_commit_embeddings {
        s.git.commit_embeddings = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Cheap RAII tempdir without pulling in the `tempfile` crate as a
    /// dev-dep on `ua-cli`. Mirrors the helper used by the other
    /// command modules.
    struct ScratchDir {
        path: PathBuf,
    }

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "ua-cli-init-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// `Args` is `clap::Args`, not `clap::Parser`. Wrap it in a
    /// throwaway `Parser` so tests can drive it from a `Vec<&str>`
    /// just like the real CLI.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn parse_args(cli: &[&str]) -> clap::error::Result<Args> {
        TestCli::try_parse_from(std::iter::once("init").chain(cli.iter().copied())).map(|c| c.args)
    }

    #[tokio::test]
    async fn dry_run_does_not_create_storage_dir() {
        let scratch = ScratchDir::new("dryrun");
        let project = scratch.path();
        let args =
            parse_args(&["--dry-run", "--no-gitignore"]).expect("clap should accept --dry-run");
        run(args, project).await.expect("dry-run must succeed");
        let storage = project.join(".understandable");
        assert!(
            !storage.exists(),
            "dry-run leaked filesystem state at {}",
            storage.display()
        );
        let yaml = project.join("understandable.yaml");
        assert!(
            !yaml.exists(),
            "dry-run wrote settings file {}",
            yaml.display()
        );
    }

    #[test]
    fn embed_provider_typo_rejected_at_parse_time() {
        // `olama` is the canonical typo from the bug report. clap must
        // reject it before we even reach `apply_flags`, so the bad
        // string never lands in the YAML.
        let err = parse_args(&["--embed-provider", "olama"])
            .expect_err("typo must be rejected by clap value-enum validation");
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
        // Sanity: a known-good value still parses.
        let ok = parse_args(&["--embed-provider", "ollama"]).expect("known provider parses");
        assert!(matches!(ok.embed_provider, Some(EmbedProviderArg::Ollama)));
    }

    #[tokio::test]
    async fn force_creates_backup_when_existing_yaml() {
        let scratch = ScratchDir::new("forcebak");
        let project = scratch.path();
        let yaml = project.join("understandable.yaml");
        std::fs::write(
            &yaml,
            "version: 1\nproject:\n  name: prior\nstorage:\n  dir: .understandable\n  db_name: graph\n",
        )
        .unwrap();
        let args = parse_args(&["--force", "--no-gitignore"]).expect("clap should accept --force");
        run(args, project).await.expect("force run must succeed");
        let bak = project.join("understandable.yaml.bak");
        assert!(bak.exists(), "expected backup file at {}", bak.display());
        let bak_contents = std::fs::read_to_string(&bak).unwrap();
        assert!(
            bak_contents.contains("name: prior"),
            "backup should hold the previous YAML contents, got: {bak_contents}"
        );
    }

    #[tokio::test]
    async fn force_merge_preserves_unrelated_yaml_fields() {
        // Set a non-default value (`project.name`) in the existing
        // YAML. Only override an unrelated field on the CLI
        // (--storage-dir). The merge must keep `project.name` intact.
        let scratch = ScratchDir::new("forcemerge");
        let project = scratch.path();
        let yaml = project.join("understandable.yaml");
        std::fs::write(
            &yaml,
            "version: 1\nproject:\n  name: keepme\nstorage:\n  dir: .understandable\n  db_name: graph\n",
        )
        .unwrap();
        let args = parse_args(&[
            "--force",
            "--storage-dir",
            ".ua-store",
            "--no-backup",
            "--no-gitignore",
        ])
        .expect("clap should accept the flags");
        run(args, project).await.expect("force merge must succeed");
        let parsed = ProjectSettings::load(project)
            .expect("post-merge YAML must parse")
            .expect("post-merge YAML must exist");
        assert_eq!(
            parsed.project.name.as_deref(),
            Some("keepme"),
            "merge dropped a hand-set field"
        );
        assert_eq!(
            parsed.storage.dir, ".ua-store",
            "CLI override for storage.dir should win"
        );
        // --no-backup should also have suppressed the .bak.
        let bak = project.join("understandable.yaml.bak");
        assert!(!bak.exists(), "--no-backup should suppress the .bak file");
    }

    #[test]
    fn backup_path_appends_extension() {
        let p = Path::new("/tmp/foo/understandable.yaml");
        assert_eq!(
            backup_path(p),
            PathBuf::from("/tmp/foo/understandable.yaml.bak")
        );
        let p = Path::new("/tmp/foo/understandable.yml");
        assert_eq!(
            backup_path(p),
            PathBuf::from("/tmp/foo/understandable.yml.bak")
        );
    }
}
