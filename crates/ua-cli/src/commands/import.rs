//! `understandable import` — load a JSON graph and replace whatever is
//! currently persisted for that kind.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use ua_core::KnowledgeGraph;
use ua_persist::{ProjectLayout, Storage};

// TODO: factor GraphKind into a shared util/ module after multi-agent wave settles.
/// Which graph slot a kind-aware command should target.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum GraphKind {
    Codebase,
    Domain,
    Knowledge,
}

impl GraphKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphKind::Codebase => "codebase",
            GraphKind::Domain => "domain",
            GraphKind::Knowledge => "knowledge",
        }
    }
}

impl fmt::Display for GraphKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<GraphKind> for &'static str {
    fn from(k: GraphKind) -> &'static str {
        k.as_str()
    }
}

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// JSON file to read (default: stdin).
    #[arg(long, default_value = "-")]
    pub r#in: PathBuf,
    /// Which graph slot to overwrite. Defaults to the kind declared in
    /// the JSON; falls back to `codebase`.
    #[arg(long, value_enum)]
    pub kind: Option<GraphKind>,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let raw = if args.r#in == std::path::Path::new("-") {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(&args.r#in)?
    };
    let graph: KnowledgeGraph = serde_json::from_str(&raw)?;

    let kind: &str = match args.kind {
        Some(k) => k.into(),
        None => match graph.kind {
            Some(ua_core::GraphKind::Codebase) | None => "codebase",
            Some(ua_core::GraphKind::Knowledge) => "knowledge",
            Some(ua_core::GraphKind::Domain) => "domain",
        },
    };

    let layout = ProjectLayout::for_project(project_path);
    layout.ensure_exists()?;
    let storage = Storage::open_kind(&layout, kind).await?;
    storage.save_graph(&graph).await?;
    storage.save_kind(&layout, kind).await?;
    println!(
        "imported into `{kind}` slot: {} nodes, {} edges, {} layers, {} tour steps",
        graph.nodes.len(),
        graph.edges.len(),
        graph.layers.len(),
        graph.tour.len(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Args, GraphKind};
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn kind_typo_rejected() {
        let res = TestCli::try_parse_from(["import", "--kind", "codebse"]);
        assert!(res.is_err(), "clap should reject `--kind codebse`");
    }

    #[test]
    fn kind_omitted_yields_none() {
        let cli = TestCli::try_parse_from(["import"]).unwrap();
        assert_eq!(cli.args.kind, None);
    }

    #[test]
    fn kind_explicit_parses() {
        let cli = TestCli::try_parse_from(["import", "--kind", "domain"]).unwrap();
        assert_eq!(cli.args.kind, Some(GraphKind::Domain));
    }
}
