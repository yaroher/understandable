//! `understandable` — single CLI binary entry point.

use clap::{Parser, Subcommand};

mod builders;
mod commands;
mod util;

#[derive(Parser, Debug)]
#[command(
    name = "understandable",
    version,
    about = "Rust-native codebase understanding — analyze, visualise, and explain any project.",
    long_about = None
)]
struct Cli {
    /// Project root. Defaults to the current working directory.
    #[arg(long, global = true, value_name = "PATH")]
    path: Option<std::path::PathBuf>,
    /// Increase log verbosity (`-v` for info, `-vv` for debug, `-vvv` for trace).
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

// Variants are Box'd where clippy::large_enum_variant flags them; the
// enum is parsed once per process so the indirection is free.
#[derive(Subcommand, Debug)]
enum Command {
    /// Run the full analysis pipeline (scan → extract → graph → persist).
    Analyze(commands::analyze::Args),
    /// Launch the interactive dashboard (phase 7).
    Dashboard(commands::dashboard::Args),
    /// Ask a free-form question over the persisted graph (phase 6).
    Chat(commands::chat::Args),
    /// Map current git changes to graph nodes (phase 6).
    Diff(commands::diff::Args),
    /// Explain a single file or symbol in detail (phase 6).
    Explain(commands::explain::Args),
    /// Generate the team onboarding guide (phase 6).
    Onboard(commands::onboard::Args),
    /// Build / refresh the domain graph (phase 10).
    Domain(commands::domain::Args),
    /// Build / refresh the knowledge-base (Karpathy wiki) graph (phase 10).
    Knowledge(commands::knowledge::Args),
    /// Run tree-sitter structural extraction over a JSON batch description.
    Extract(commands::extract::Args),
    /// Merge multiple intermediate JSON outputs into one (phase 9).
    Merge(commands::merge::Args),
    /// Validate the persisted graph against the core schema.
    Validate(commands::validate::Args),
    /// Compare the persisted graph's git_commit_hash against current HEAD.
    /// Exit codes: 0 fresh · 1 stale · 2 no graph · 3 error.
    Staleness(commands::staleness::Args),
    /// Update file fingerprints stored in the graph.
    Fingerprint(commands::fingerprint::Args),
    /// Export the persisted graph as JSON (stdout or `--out`).
    Export(commands::export::Args),
    /// Import a JSON graph into the database (replaces existing).
    Import(commands::import::Args),
    /// Search graph nodes (substring + node-type filter).
    Search(commands::search::Args),
    /// Bulk-embed graph nodes and store vectors for `search --semantic`.
    Embed(commands::embed::Args),
    /// Scaffold a project-level `understandable.yaml`.
    Init(Box<commands::init::Args>),
    /// Bootstrap project-local artefacts (`.understandignore`, …).
    Scan(commands::scan::Args),
}

fn init_tracing(verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let project_path = cli
        .path
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));

    match cli.command {
        Command::Analyze(args) => commands::analyze::run(args, &project_path).await,
        Command::Dashboard(args) => commands::dashboard::run(args, &project_path).await,
        Command::Chat(args) => commands::chat::run(args, &project_path).await,
        Command::Diff(args) => commands::diff::run(args, &project_path).await,
        Command::Explain(args) => commands::explain::run(args, &project_path).await,
        Command::Onboard(args) => commands::onboard::run(args, &project_path).await,
        Command::Domain(args) => commands::domain::run(args, &project_path).await,
        Command::Knowledge(args) => commands::knowledge::run(args, &project_path).await,
        Command::Extract(args) => commands::extract::run(args).await,
        Command::Merge(args) => commands::merge::run(args).await,
        Command::Validate(args) => commands::validate::run(args, &project_path).await,
        Command::Staleness(args) => commands::staleness::run(args, &project_path).await,
        Command::Fingerprint(args) => commands::fingerprint::run(args, &project_path).await,
        Command::Export(args) => commands::export::run(args, &project_path).await,
        Command::Import(args) => commands::import::run(args, &project_path).await,
        Command::Search(args) => commands::search::run(args, &project_path).await,
        Command::Embed(args) => commands::embed::run(args, &project_path).await,
        Command::Init(args) => commands::init::run(*args, &project_path).await,
        Command::Scan(args) => commands::scan::run(args, &project_path).await,
    }
}
