//! Heuristic domain-graph extractor.
//!
//! Produces `domain` / `flow` / `step` nodes and the corresponding
//! `contains_flow` / `flow_step` / `cross_domain` edges from an
//! existing codebase graph. The LLM-driven `domain-analyzer` agent
//! enriches the result; this module guarantees a well-formed
//! deterministic substrate so the agent always has something to work
//! with.

use std::collections::{BTreeMap, BTreeSet};

use ua_core::{
    DomainEntryType, DomainMeta, EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode,
    KnowledgeGraph, NodeType,
};

/// Top-level directory under `src/` (or project root) is treated as a
/// domain, every file inside is a flow, and every function/class
/// inside a file is a step.
pub fn build_domain_graph(graph: &KnowledgeGraph) -> KnowledgeGraph {
    let mut domains: BTreeMap<String, Vec<&GraphNode>> = BTreeMap::new();
    for node in &graph.nodes {
        if node.node_type != NodeType::File {
            continue;
        }
        let Some(path) = &node.file_path else {
            continue;
        };
        let domain = top_level_domain(path);
        domains.entry(domain).or_default().push(node);
    }

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_edge: BTreeSet<String> = BTreeSet::new();
    let mut push_edge = |edges: &mut Vec<GraphEdge>, e: GraphEdge| {
        let key = format!("{}|{}|{:?}", e.source, e.target, e.edge_type);
        if seen_edge.insert(key) {
            edges.push(e);
        }
    };

    for (domain_name, files) in &domains {
        let domain_id = format!("domain:{domain_name}");
        let entities: Vec<String> = files
            .iter()
            .filter_map(|n| n.file_path.as_deref().map(|p| {
                p.rsplit('/').next().unwrap_or(p).to_string()
            }))
            .collect();
        nodes.push(GraphNode {
            id: domain_id.clone(),
            node_type: NodeType::Domain,
            name: domain_name.clone(),
            file_path: None,
            line_range: None,
            summary: format!(
                "Heuristic domain inferred from `{domain_name}/` ({} files).",
                files.len()
            ),
            tags: vec!["heuristic".into(), "domain".into()],
            complexity: ua_core::Complexity::Moderate,
            language_notes: None,
            domain_meta: Some(DomainMeta {
                entities: Some(entities),
                business_rules: None,
                cross_domain_interactions: None,
                entry_point: None,
                entry_type: None,
            }),
            knowledge_meta: None,
        });

        for file in files {
            let flow_id = format!("flow:{}", file.id.trim_start_matches("file:"));
            let entry_type = guess_entry_type(file);
            nodes.push(GraphNode {
                id: flow_id.clone(),
                node_type: NodeType::Flow,
                name: file.name.clone(),
                file_path: file.file_path.clone(),
                line_range: None,
                summary: file.summary.clone(),
                tags: vec!["heuristic".into(), "flow".into()],
                complexity: file.complexity,
                language_notes: None,
                domain_meta: Some(DomainMeta {
                    entities: None,
                    business_rules: None,
                    cross_domain_interactions: None,
                    entry_point: file.file_path.clone(),
                    entry_type: entry_type.into(),
                }),
                knowledge_meta: None,
            });
            push_edge(
                &mut edges,
                GraphEdge {
                    source: domain_id.clone(),
                    target: flow_id.clone(),
                    edge_type: EdgeType::ContainsFlow,
                    direction: EdgeDirection::Forward,
                    description: None,
                    weight: 1.0,
                },
            );

            // Promote functions / classes inside the file into step nodes.
            for child in &graph.nodes {
                let is_child_of_file = matches!(
                    child.node_type,
                    NodeType::Function | NodeType::Class
                ) && child.file_path == file.file_path;
                if !is_child_of_file {
                    continue;
                }
                let step_slug: String = child
                    .name
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_whitespace() { '-' } else { c })
                    .collect();
                let step_id = format!(
                    "step:{}:{}:{}",
                    domain_name,
                    file.file_path.as_deref().unwrap_or(""),
                    step_slug
                );
                nodes.push(GraphNode {
                    id: step_id.clone(),
                    node_type: NodeType::Step,
                    name: child.name.clone(),
                    file_path: child.file_path.clone(),
                    line_range: child.line_range,
                    summary: child.summary.clone(),
                    tags: vec!["heuristic".into(), "step".into()],
                    complexity: child.complexity,
                    language_notes: None,
                    domain_meta: None,
                    knowledge_meta: None,
                });
                push_edge(
                    &mut edges,
                    GraphEdge {
                        source: flow_id.clone(),
                        target: step_id,
                        edge_type: EdgeType::FlowStep,
                        direction: EdgeDirection::Forward,
                        description: None,
                        weight: 1.0,
                    },
                );
            }
        }
    }

    // Cross-domain edges: file A imports file B where they belong to
    // different inferred domains.
    for e in &graph.edges {
        if e.edge_type != EdgeType::Imports {
            continue;
        }
        let src = file_domain(graph, &e.source);
        let tgt = file_domain(graph, &e.target);
        if let (Some(a), Some(b)) = (src, tgt) {
            if a != b {
                push_edge(
                    &mut edges,
                    GraphEdge {
                        source: format!("domain:{a}"),
                        target: format!("domain:{b}"),
                        edge_type: EdgeType::CrossDomain,
                        direction: EdgeDirection::Forward,
                        description: Some("cross-domain import".into()),
                        weight: 0.5,
                    },
                );
            }
        }
    }

    KnowledgeGraph {
        version: env!("CARGO_PKG_VERSION").to_string(),
        kind: Some(GraphKind::Domain),
        project: graph.project.clone(),
        nodes,
        edges,
        layers: Vec::new(),
        tour: Vec::new(),
    }
}

fn top_level_domain(file_path: &str) -> String {
    let normalised = file_path.trim_start_matches('/').replace('\\', "/");
    let parts: Vec<&str> = normalised.split('/').collect();
    // Common pattern: `src/<domain>/...`. Strip a leading `src` if present.
    if parts.len() >= 2 && (parts[0] == "src" || parts[0] == "lib" || parts[0] == "app") {
        parts[1].to_string()
    } else if parts.len() >= 2 {
        parts[0].to_string()
    } else {
        "root".to_string()
    }
}

fn file_domain(graph: &KnowledgeGraph, node_id: &str) -> Option<String> {
    let node = graph.nodes.iter().find(|n| n.id == node_id)?;
    node.file_path.as_deref().map(top_level_domain)
}

fn guess_entry_type(file: &GraphNode) -> Option<DomainEntryType> {
    let path = file.file_path.as_deref()?.to_lowercase();
    if path.contains("routes")
        || path.contains("handlers")
        || path.contains("controllers")
        || path.contains("/api/")
    {
        Some(DomainEntryType::Http)
    } else if path.contains("/cli") || path.contains("commands") {
        Some(DomainEntryType::Cli)
    } else if path.contains("worker") || path.contains("queue") || path.contains("consumer") {
        Some(DomainEntryType::Event)
    } else if path.contains("cron") || path.contains("scheduler") {
        Some(DomainEntryType::Cron)
    } else {
        None
    }
}
