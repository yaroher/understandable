//! Chat-context builder — port of `src/context-builder.ts`.

use std::collections::{BTreeMap, HashSet};

use ua_core::{GraphEdge, GraphNode, KnowledgeGraph, Layer};

use crate::engine::{SearchEngine, SearchOptions};

#[derive(Debug, Clone)]
pub struct ChatContext {
    pub project_name: String,
    pub project_description: String,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub relevant_nodes: Vec<GraphNode>,
    pub relevant_edges: Vec<GraphEdge>,
    pub relevant_layers: Vec<Layer>,
    pub query: String,
}

/// Search the graph, expand 1 hop via edges, and collect related layers.
pub fn build_chat_context(graph: &KnowledgeGraph, query: &str, max_nodes: usize) -> ChatContext {
    let limit = if max_nodes == 0 { 15 } else { max_nodes };
    let engine = SearchEngine::new(graph.nodes.clone());
    let results = engine.search(
        query,
        &SearchOptions {
            limit: Some(limit),
            ..Default::default()
        },
    );
    let matched: HashSet<String> = results.into_iter().map(|r| r.node_id).collect();

    // 1-hop expansion via any edge.
    let mut expanded = matched.clone();
    for e in &graph.edges {
        if matched.contains(&e.source) {
            expanded.insert(e.target.clone());
        }
        if matched.contains(&e.target) {
            expanded.insert(e.source.clone());
        }
    }

    // Stable order so prompts diff cleanly across runs.
    let node_map: BTreeMap<&str, &GraphNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut relevant_nodes: Vec<GraphNode> = expanded
        .iter()
        .filter_map(|id| node_map.get(id.as_str()).map(|n| (*n).clone()))
        .collect();
    relevant_nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let relevant_edges: Vec<GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| expanded.contains(&e.source) && expanded.contains(&e.target))
        .cloned()
        .collect();

    let relevant_layers: Vec<Layer> = graph
        .layers
        .iter()
        .filter(|l| l.node_ids.iter().any(|id| expanded.contains(id)))
        .cloned()
        .collect();

    ChatContext {
        project_name: graph.project.name.clone(),
        project_description: graph.project.description.clone(),
        languages: graph.project.languages.clone(),
        frameworks: graph.project.frameworks.clone(),
        relevant_nodes,
        relevant_edges,
        relevant_layers,
        query: query.to_string(),
    }
}

/// Render the context as markdown — mirrors `formatContextForPrompt`.
pub fn format_context_for_prompt(ctx: &ChatContext) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# Project: {}", ctx.project_name));
    lines.push(String::new());
    lines.push(ctx.project_description.clone());
    lines.push(String::new());
    lines.push(format!("**Languages:** {}", ctx.languages.join(", ")));
    lines.push(format!("**Frameworks:** {}", ctx.frameworks.join(", ")));
    lines.push(String::new());

    if !ctx.relevant_layers.is_empty() {
        lines.push("## Relevant Layers".into());
        lines.push(String::new());
        for l in &ctx.relevant_layers {
            lines.push(format!("### {}", l.name));
            lines.push(l.description.clone());
            lines.push(String::new());
        }
    }

    if !ctx.relevant_nodes.is_empty() {
        lines.push("## Code Components".into());
        lines.push(String::new());
        for n in &ctx.relevant_nodes {
            lines.push(format!(
                "### {} ({})",
                n.name,
                n.node_type.as_str()
            ));
            if let Some(p) = &n.file_path {
                lines.push(format!("- **File:** {p}"));
            }
            lines.push(format!(
                "- **Complexity:** {}",
                match n.complexity {
                    ua_core::Complexity::Simple => "simple",
                    ua_core::Complexity::Moderate => "moderate",
                    ua_core::Complexity::Complex => "complex",
                }
            ));
            lines.push(format!("- **Summary:** {}", n.summary));
            if !n.tags.is_empty() {
                lines.push(format!("- **Tags:** {}", n.tags.join(", ")));
            }
            if let Some(notes) = &n.language_notes {
                lines.push(format!("- **Language Notes:** {notes}"));
            }
            lines.push(String::new());
        }
    }

    if !ctx.relevant_edges.is_empty() {
        let by_id: BTreeMap<&str, &GraphNode> =
            ctx.relevant_nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        lines.push("## Relationships".into());
        lines.push(String::new());
        for e in &ctx.relevant_edges {
            let src = by_id.get(e.source.as_str()).map(|n| n.name.as_str()).unwrap_or(&e.source);
            let tgt = by_id.get(e.target.as_str()).map(|n| n.name.as_str()).unwrap_or(&e.target);
            let mut line = format!("- {src} --[{}]--> {tgt}", edge_type_label(e.edge_type));
            if let Some(d) = &e.description {
                line.push_str(": ");
                line.push_str(d);
            }
            lines.push(line);
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

fn edge_type_label(t: ua_core::EdgeType) -> &'static str {
    use ua_core::EdgeType::*;
    match t {
        Imports => "imports",
        Exports => "exports",
        Contains => "contains",
        Inherits => "inherits",
        Implements => "implements",
        Calls => "calls",
        Subscribes => "subscribes",
        Publishes => "publishes",
        Middleware => "middleware",
        ReadsFrom => "reads_from",
        WritesTo => "writes_to",
        Transforms => "transforms",
        Validates => "validates",
        DependsOn => "depends_on",
        TestedBy => "tested_by",
        Configures => "configures",
        Related => "related",
        SimilarTo => "similar_to",
        Deploys => "deploys",
        Serves => "serves",
        Provisions => "provisions",
        Triggers => "triggers",
        Migrates => "migrates",
        Documents => "documents",
        Routes => "routes",
        DefinesSchema => "defines_schema",
        ContainsFlow => "contains_flow",
        FlowStep => "flow_step",
        CrossDomain => "cross_domain",
        Cites => "cites",
        Contradicts => "contradicts",
        BuildsOn => "builds_on",
        Exemplifies => "exemplifies",
        CategorizedUnder => "categorized_under",
        AuthoredBy => "authored_by",
    }
}
