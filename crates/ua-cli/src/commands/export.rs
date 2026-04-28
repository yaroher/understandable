//! `understandable export` — dump the persisted graph as JSON.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
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
    /// Output path (default: stdout).
    #[arg(long, default_value = "-")]
    pub out: PathBuf,
    /// Pretty-print the JSON.
    #[arg(long)]
    pub pretty: bool,
    /// Which graph to export. Defaults to the codebase graph.
    #[arg(long, value_enum, default_value_t = GraphKind::Codebase)]
    pub kind: GraphKind,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project_path);
    let kind: &str = args.kind.into();
    let storage = Storage::open_kind(&layout, kind).await?;
    let graph = storage.load_graph().await?;
    let json = if args.pretty {
        serde_json::to_string_pretty(&graph)?
    } else {
        serde_json::to_string(&graph)?
    };
    if args.out == PathBuf::from("-") {
        std::io::stdout().write_all(json.as_bytes())?;
        std::io::stdout().write_all(b"\n")?;
    } else {
        std::fs::write(&args.out, json)?;
    }
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
        let res = TestCli::try_parse_from(["export", "--kind", "codebse"]);
        assert!(res.is_err(), "clap should reject `--kind codebse`");
    }

    #[test]
    fn kind_default_is_codebase() {
        let cli = TestCli::try_parse_from(["export"]).unwrap();
        assert_eq!(cli.args.kind, GraphKind::Codebase);
    }
}
