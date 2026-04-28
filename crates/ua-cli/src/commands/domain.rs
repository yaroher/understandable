//! `understandable domain` — derive the deterministic domain/flow/step
//! substrate from the persisted codebase graph and write it to
//! `.understandable/graph.domain.tar.zst`.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_analyzer::build_domain_graph;
use ua_persist::{ProjectLayout, Storage};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Force a full rebuild even if a domain graph already exists.
    #[arg(long)]
    pub full: bool,
}

pub async fn run(_args: Args, project: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project);
    let codebase = Storage::open_kind(&layout, "codebase").await?;
    let codebase_graph = codebase.load_graph().await?;
    if codebase_graph.nodes.is_empty() {
        anyhow::bail!("no codebase graph found — run `understandable analyze` first");
    }
    let domain_graph = build_domain_graph(&codebase_graph);
    let storage = Storage::open_kind(&layout, "domain").await?;
    storage.save_graph(&domain_graph).await?;
    storage.save_kind(&layout, "domain").await?;
    println!(
        "domain graph: {} nodes ({} domains, {} flows, {} steps), {} edges",
        domain_graph.nodes.len(),
        count_kind(&domain_graph, ua_core::NodeType::Domain),
        count_kind(&domain_graph, ua_core::NodeType::Flow),
        count_kind(&domain_graph, ua_core::NodeType::Step),
        domain_graph.edges.len(),
    );
    Ok(())
}

fn count_kind(g: &ua_core::KnowledgeGraph, k: ua_core::NodeType) -> usize {
    g.nodes.iter().filter(|n| n.node_type == k).count()
}
