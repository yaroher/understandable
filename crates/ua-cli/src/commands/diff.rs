//! `understandable diff` — map current git changes to the graph.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_persist::{ProjectLayout, Storage};

use crate::builders::diff_analyzer::{
    build_diff_context, changed_files_from_git, format_diff_analysis,
};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Explicit file list (space-separated). If omitted, the binary
    /// calls `git status --porcelain` and uses every staged + unstaged
    /// path. Pass each path as its own value:
    /// `understandable diff --files src/auth.ts src/api.ts`.
    #[arg(long, num_args = 1..)]
    pub files: Vec<String>,
    /// Skip writing `.understandable/diff-overlay.json`. By default the
    /// binary persists the diff context as JSON next to the graph so
    /// `/api/diff` (and the dashboard) can pick it up. Use this when
    /// you only want the human-readable text on stdout.
    #[arg(long)]
    pub no_write: bool,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let files = if args.files.is_empty() {
        changed_files_from_git(project)
    } else {
        args.files
    };
    if files.is_empty() {
        println!("no changed files detected");
        return Ok(());
    }
    let layout = ProjectLayout::for_project(project);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let ctx = build_diff_context(&graph, &files);
    println!("{}", format_diff_analysis(&ctx));

    if !args.no_write {
        // Persist the structured diff context next to the graph so
        // `/api/diff` (read by the dashboard) returns 200 instead of
        // 204. Atomic write via tmp + rename so a crashed run never
        // leaves a half-written overlay.
        layout.ensure_exists()?;
        let dst = layout.root.join("diff-overlay.json");
        let tmp = layout.root.join("diff-overlay.json.tmp");
        let bytes = serde_json::to_vec_pretty(&ctx)?;
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &dst)?;
        println!("[diff] wrote {}", dst.display());
    }

    Ok(())
}
