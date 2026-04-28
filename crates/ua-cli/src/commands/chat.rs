//! `understandable chat` — render the LLM-ready chat context for a query.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_persist::{ProjectLayout, Storage};
use ua_search::{build_chat_context, format_context_for_prompt};

#[derive(ClapArgs, Debug)]
pub struct Args {
    pub query: String,
    /// Maximum number of seed nodes returned by search.
    #[arg(long, default_value_t = 15)]
    pub limit: usize,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let ctx = build_chat_context(&graph, &args.query, args.limit);
    println!("{}", format_context_for_prompt(&ctx));
    Ok(())
}
