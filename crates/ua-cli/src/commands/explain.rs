//! `understandable explain` — deep-dive prompt for a file or `path:symbol`.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_persist::{ProjectLayout, Storage};

use crate::builders::explain_builder::{build_explain_context, format_explain_prompt};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// File path or `path:symbol` (e.g. `src/auth.ts:login`).
    pub target: String,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let ctx = build_explain_context(&graph, &args.target);
    println!("{}", format_explain_prompt(&ctx));
    Ok(())
}
