//! Generate a markdown onboarding guide from a knowledge graph.

use ua_core::{Complexity, KnowledgeGraph, NodeType};

pub fn build_onboarding_guide(graph: &KnowledgeGraph) -> String {
    let mut lines: Vec<String> = Vec::new();
    let p = &graph.project;

    lines.push(format!("# {}", p.name));
    lines.push(String::new());
    lines.push(format!("> {}", p.description));
    lines.push(String::new());
    lines.push("| | |".into());
    lines.push("|---|---|".into());
    lines.push(format!("| **Languages** | {} |", p.languages.join(", ")));
    lines.push(format!("| **Frameworks** | {} |", p.frameworks.join(", ")));
    lines.push(format!(
        "| **Components** | {} nodes, {} relationships |",
        graph.nodes.len(),
        graph.edges.len()
    ));
    lines.push(format!("| **Last Analyzed** | {} |", p.analyzed_at));
    lines.push(String::new());

    if !graph.layers.is_empty() {
        lines.push("## Architecture".into());
        lines.push(String::new());
        lines.push("The project is organized into the following layers:".into());
        lines.push(String::new());
        for l in &graph.layers {
            let names: Vec<&str> = l
                .node_ids
                .iter()
                .filter_map(|id| {
                    graph
                        .nodes
                        .iter()
                        .find(|n| &n.id == id)
                        .map(|n| n.name.as_str())
                })
                .collect();
            lines.push(format!("### {}", l.name));
            lines.push(String::new());
            lines.push(l.description.clone());
            lines.push(String::new());
            if !names.is_empty() {
                lines.push(format!("Key components: {}", names.join(", ")));
                lines.push(String::new());
            }
        }
    }

    let concepts: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Concept)
        .collect();
    if !concepts.is_empty() {
        lines.push("## Key Concepts".into());
        lines.push(String::new());
        lines.push("Important architectural and domain concepts to understand:".into());
        lines.push(String::new());
        for c in concepts {
            lines.push(format!("### {}", c.name));
            lines.push(String::new());
            lines.push(c.summary.clone());
            lines.push(String::new());
        }
    }

    if !graph.tour.is_empty() {
        lines.push("## Getting Started".into());
        lines.push(String::new());
        lines.push("Follow this guided tour to understand the codebase:".into());
        lines.push(String::new());
        for step in &graph.tour {
            let step_nodes: Vec<_> = step
                .node_ids
                .iter()
                .filter_map(|id| graph.nodes.iter().find(|n| &n.id == id))
                .collect();
            lines.push(format!("### {}. {}", step.order, step.title));
            lines.push(String::new());
            lines.push(step.description.clone());
            lines.push(String::new());
            if !step_nodes.is_empty() {
                lines.push("**Files to look at:**".into());
                for n in &step_nodes {
                    if let Some(p) = &n.file_path {
                        lines.push(format!("- `{p}` — {}", n.summary));
                    }
                }
                lines.push(String::new());
            }
            if let Some(lesson) = &step.language_lesson {
                lines.push(format!("> **Language Tip:** {lesson}"));
                lines.push(String::new());
            }
        }
    }

    let files: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::File && n.file_path.is_some())
        .collect();
    if !files.is_empty() {
        lines.push("## File Map".into());
        lines.push(String::new());
        lines.push("| File | Purpose | Complexity |".into());
        lines.push("|------|---------|------------|".into());
        for n in files {
            lines.push(format!(
                "| `{}` | {} | {} |",
                n.file_path.as_deref().unwrap_or(""),
                n.summary,
                complexity_label(n.complexity)
            ));
        }
        lines.push(String::new());
    }

    let hotspots: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.complexity == Complexity::Complex)
        .collect();
    if !hotspots.is_empty() {
        lines.push("## Complexity Hotspots".into());
        lines.push(String::new());
        lines.push("These components are the most complex and deserve extra attention:".into());
        lines.push(String::new());
        for n in hotspots {
            lines.push(format!(
                "- **{}** ({}): {}",
                n.name,
                n.node_type.as_str(),
                n.summary
            ));
        }
        lines.push(String::new());
    }

    lines.push("---".into());
    lines.push(String::new());
    lines.push(format!(
        "*Generated by [understandable](https://github.com/yaroher/understandable) from knowledge graph v{}*",
        graph.version
    ));
    lines.push(String::new());
    lines.join("\n")
}

fn complexity_label(c: Complexity) -> &'static str {
    match c {
        Complexity::Simple => "simple",
        Complexity::Moderate => "moderate",
        Complexity::Complex => "complex",
    }
}
