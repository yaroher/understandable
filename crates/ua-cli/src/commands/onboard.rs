//! `understandable onboard` — generate a markdown onboarding guide.

use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use ua_persist::{ProjectLayout, Storage};

use crate::builders::onboard_builder::build_onboarding_guide;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Output file. Defaults to stdout.
    #[arg(long, default_value = "-")]
    pub out: PathBuf,
}

pub async fn run(args: Args, project: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let md = build_onboarding_guide(&graph);
    if args.out == PathBuf::from("-") {
        println!("{md}");
    } else {
        std::fs::write(&args.out, md)?;
    }
    Ok(())
}
