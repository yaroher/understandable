//! Build a deep-dive prompt for a file or `path:symbol`.

use std::collections::{BTreeMap, HashSet};

use ua_core::{Complexity, EdgeType, GraphEdge, GraphNode, KnowledgeGraph, Layer, NodeType};

#[derive(Debug, Clone)]
pub struct ExplainContext {
    pub project_name: String,
    pub path: String,
    pub target_node: Option<GraphNode>,
    pub child_nodes: Vec<GraphNode>,
    pub connected_nodes: Vec<GraphNode>,
    pub relevant_edges: Vec<GraphEdge>,
    pub layer: Option<Layer>,
}

pub fn build_explain_context(graph: &KnowledgeGraph, path: &str) -> ExplainContext {
    // 1. `path:symbol` (e.g. `src/auth.ts:login`) — only when the part
    //    after the last colon doesn't look like a URL scheme.
    let mut target_node: Option<GraphNode> = None;
    if let Some(colon_idx) = path.rfind(':') {
        if !path.contains("://") && colon_idx > 0 {
            let file_path = &path[..colon_idx];
            let func_name = &path[colon_idx + 1..];
            target_node = graph
                .nodes
                .iter()
                .find(|n| n.file_path.as_deref() == Some(file_path) && n.name == func_name)
                .cloned();
        }
    }
    if target_node.is_none() {
        target_node = graph
            .nodes
            .iter()
            .find(|n| n.file_path.as_deref() == Some(path))
            .cloned();
    }

    let Some(target) = target_node else {
        return ExplainContext {
            project_name: graph.project.name.clone(),
            path: path.to_string(),
            target_node: None,
            child_nodes: Vec::new(),
            connected_nodes: Vec::new(),
            relevant_edges: Vec::new(),
            layer: None,
        };
    };

    let target_id = target.id.clone();
    let child_nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| {
            graph.edges.iter().any(|e| {
                e.source == target_id && e.target == n.id && e.edge_type == EdgeType::Contains
            })
        })
        .cloned()
        .collect();

    let mut all_related: HashSet<String> = HashSet::new();
    all_related.insert(target_id.clone());
    for c in &child_nodes {
        all_related.insert(c.id.clone());
    }

    let mut connected_ids: HashSet<String> = HashSet::new();
    let mut relevant_edges: Vec<GraphEdge> = Vec::new();
    for e in &graph.edges {
        let src_in = all_related.contains(&e.source);
        let tgt_in = all_related.contains(&e.target);
        if src_in || tgt_in {
            relevant_edges.push(e.clone());
            if src_in && !tgt_in {
                connected_ids.insert(e.target.clone());
            }
            if tgt_in && !src_in {
                connected_ids.insert(e.source.clone());
            }
        }
    }

    let connected_nodes: Vec<GraphNode> = graph
        .nodes
        .iter()
        .filter(|n| connected_ids.contains(&n.id))
        .cloned()
        .collect();

    let layer = graph
        .layers
        .iter()
        .find(|l| l.node_ids.contains(&target_id))
        .cloned();

    ExplainContext {
        project_name: graph.project.name.clone(),
        path: path.to_string(),
        target_node: Some(target),
        child_nodes,
        connected_nodes,
        relevant_edges,
        layer,
    }
}

pub fn format_explain_prompt(ctx: &ExplainContext) -> String {
    let Some(target) = &ctx.target_node else {
        return [
            "# Component Not Found".to_string(),
            String::new(),
            format!(
                "The path \"{}\" was not found in the knowledge graph for {}.",
                ctx.path, ctx.project_name
            ),
            String::new(),
            "Possible reasons:".into(),
            "- The file hasn't been analyzed yet — try running `understandable analyze` first".into(),
            "- The path may be different in the graph — check the exact file path".into(),
            "- The file may have been deleted or renamed since the last analysis".into(),
        ]
        .join("\n");
    };

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("# Deep Dive: {}", target.name));
    lines.push(String::new());
    lines.push(format!(
        "**Type:** {} | **Complexity:** {}",
        target.node_type.as_str(),
        complexity_label(target.complexity)
    ));
    if let Some(p) = &target.file_path {
        lines.push(format!("**File:** `{p}`"));
    }
    if let Some((s, e)) = target.line_range {
        lines.push(format!("**Lines:** {s}-{e}"));
    }
    lines.push(String::new());
    lines.push(format!("**Summary:** {}", target.summary));
    lines.push(String::new());

    if let Some(l) = &ctx.layer {
        lines.push(format!("## Architectural Layer: {}", l.name));
        lines.push(l.description.clone());
        lines.push(String::new());
    }

    if !ctx.child_nodes.is_empty() {
        lines.push("## Internal Components".into());
        for c in &ctx.child_nodes {
            lines.push(format!(
                "- **{}** ({}): {}",
                c.name,
                c.node_type.as_str(),
                c.summary
            ));
        }
        lines.push(String::new());
    }

    if !ctx.connected_nodes.is_empty() {
        lines.push("## Connected Components".into());
        for c in &ctx.connected_nodes {
            lines.push(format!(
                "- **{}** ({}): {}",
                c.name,
                c.node_type.as_str(),
                c.summary
            ));
        }
        lines.push(String::new());
    }

    if !ctx.relevant_edges.is_empty() {
        let by_id: BTreeMap<&str, &GraphNode> = std::iter::once(target)
            .chain(ctx.child_nodes.iter())
            .chain(ctx.connected_nodes.iter())
            .map(|n| (n.id.as_str(), n))
            .collect();
        lines.push("## Relationships".into());
        for e in &ctx.relevant_edges {
            if e.edge_type == EdgeType::Contains {
                continue;
            }
            let src = by_id
                .get(e.source.as_str())
                .map(|n| n.name.as_str())
                .unwrap_or(&e.source);
            let tgt = by_id
                .get(e.target.as_str())
                .map(|n| n.name.as_str())
                .unwrap_or(&e.target);
            let mut line = format!(
                "- {src} --[{}]--> {tgt}",
                edge_type_label(e.edge_type)
            );
            if let Some(d) = &e.description {
                line.push_str(" — ");
                line.push_str(d);
            }
            lines.push(line);
        }
        lines.push(String::new());
    }

    if let Some(notes) = &target.language_notes {
        lines.push("## Language Notes".into());
        lines.push(notes.clone());
        lines.push(String::new());
    }

    lines.push("## Instructions".into());
    lines.push("Provide a thorough explanation of this component:".into());
    lines.push("1. What it does and why it exists in the project".into());
    lines.push("2. How data flows through it (inputs, processing, outputs)".into());
    lines.push("3. How it interacts with connected components".into());
    lines.push("4. Any patterns, idioms, or design decisions worth noting".into());
    lines.push("5. Potential gotchas or areas of complexity".into());
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

fn edge_type_label(t: EdgeType) -> &'static str {
    use EdgeType::*;
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

// Quiet warnings about NodeType being only re-exported for callers.
#[allow(dead_code)]
fn _node_type_used(_: NodeType) {}
