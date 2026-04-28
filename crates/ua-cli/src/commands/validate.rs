//! `understandable validate` — run schema + referential checks against
//! the persisted graph.
//!
//! Two output modes:
//! - default: human-readable text (back-compat with the original behaviour).
//! - `--json`: emit `{ valid, issues, warnings, stats }` to stdout, matching
//!   the TS validator contract documented in `skills/understand/SKILL.md`
//!   lines 480-577.
//!
//! Exit codes:
//! - `0` if no errors (warnings allowed).
//! - `1` if any errors. With `--strict`, also `1` if any warnings.

use std::path::Path;

use clap::Args as ClapArgs;
use ua_core::validate_graph;
use ua_persist::{ProjectLayout, Storage};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Emit machine-readable JSON to stdout instead of human text.
    #[arg(long)]
    pub json: bool,
    /// Treat warnings as failures (exit 1 on any warning).
    #[arg(long)]
    pub strict: bool,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project_path);
    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let report = validate_graph(&graph);

    if args.json {
        // Single deterministic JSON document. Use pretty-print so the
        // output is diff-friendly and human-skimmable.
        let out = serde_json::to_string_pretty(&report)?;
        println!("{out}");
    } else {
        if report.is_valid() {
            println!(
                "graph valid — {} nodes, {} edges, {} layers, {} tour steps",
                graph.nodes.len(),
                graph.edges.len(),
                graph.layers.len(),
                graph.tour.len(),
            );
        } else {
            println!("graph invalid: {} issue(s)", report.issues.len());
            for issue in &report.issues {
                println!("  - [{}] {}", issue.code, issue.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("{} warning(s):", report.warnings.len());
            for w in &report.warnings {
                println!("  - [{}] {}", w.code, w.message);
            }
        }
    }

    let has_errors = !report.is_valid();
    let has_warnings = !report.warnings.is_empty();
    if has_errors {
        anyhow::bail!("validation failed");
    }
    if args.strict && has_warnings {
        anyhow::bail!("validation failed (strict): {} warning(s)", report.warnings.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ua_core::validate::codes;
    use ua_core::{
        Complexity, EdgeDirection, EdgeType, GraphEdge, GraphNode, KnowledgeGraph, NodeType,
        ProjectMeta,
    };

    fn tiny_graph() -> KnowledgeGraph {
        KnowledgeGraph {
            version: "0".into(),
            kind: None,
            project: ProjectMeta::default(),
            nodes: vec![
                GraphNode {
                    id: "a".into(),
                    node_type: NodeType::Function,
                    name: "a".into(),
                    file_path: None,
                    line_range: None,
                    summary: "s".into(),
                    tags: vec!["t".into()],
                    complexity: Complexity::Simple,
                    language_notes: None,
                    domain_meta: None,
                    knowledge_meta: None,
                },
                GraphNode {
                    id: "b".into(),
                    node_type: NodeType::Function,
                    name: "b".into(),
                    file_path: None,
                    line_range: None,
                    summary: "s".into(),
                    tags: vec!["t".into()],
                    complexity: Complexity::Simple,
                    language_notes: None,
                    domain_meta: None,
                    knowledge_meta: None,
                },
            ],
            edges: vec![GraphEdge {
                source: "a".into(),
                target: "b".into(),
                edge_type: EdgeType::Imports,
                direction: EdgeDirection::Forward,
                description: None,
                weight: 0.5,
            }],
            layers: vec![],
            tour: vec![],
        }
    }

    /// Smoke test: the report serialises to JSON that contains the
    /// publicly-documented top-level keys. The CLI just calls
    /// `serde_json::to_string_pretty(&report)` on this, so if the shape
    /// is right here, the `--json` path is right.
    #[test]
    fn json_flag_emits_valid_json() {
        let g = tiny_graph();
        let r = validate_graph(&g);
        let s = serde_json::to_string(&r).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("parse");
        assert_eq!(v.get("valid").and_then(|x| x.as_bool()), Some(true));
        assert!(v.get("issues").unwrap().is_array());
        assert!(v.get("warnings").unwrap().is_array());
        assert!(v.get("stats").unwrap().is_object());
        // Codes module is reachable from the CLI crate (round-trip on the
        // public API surface).
        assert_eq!(codes::ORPHAN_NODE, "ORPHAN_NODE");
    }
}
