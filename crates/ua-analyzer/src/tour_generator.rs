//! Heuristic tour generation — port of `analyzer/tour-generator.ts`.
//!
//! Strategy:
//!   1. separate `concept` nodes from code nodes;
//!   2. build adjacency for code nodes only;
//!   3. Kahn topological sort;
//!   4. group by layer when layers exist, else batch by 3;
//!   5. append a "Key Concepts" step for any concept nodes.

use std::collections::{HashMap, HashSet, VecDeque};

use ua_core::{KnowledgeGraph, NodeType, TourStep};

pub fn generate_heuristic_tour(graph: &KnowledgeGraph) -> Vec<TourStep> {
    let concept_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Concept)
        .collect();
    let code_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.node_type != NodeType::Concept)
        .collect();
    let code_ids: HashSet<&str> = code_nodes.iter().map(|n| n.id.as_str()).collect();

    let mut in_degree: HashMap<&str, u32> = HashMap::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in &code_nodes {
        in_degree.insert(n.id.as_str(), 0);
        adjacency.insert(n.id.as_str(), Vec::new());
    }
    for e in &graph.edges {
        if !code_ids.contains(e.source.as_str()) || !code_ids.contains(e.target.as_str()) {
            continue;
        }
        *in_degree.entry(e.target.as_str()).or_insert(0) += 1;
        adjacency
            .entry(e.source.as_str())
            .or_default()
            .push(e.target.as_str());
    }

    let mut queue: VecDeque<&str> = VecDeque::new();
    for (id, deg) in &in_degree {
        if *deg == 0 {
            queue.push_back(id);
        }
    }
    let mut topo: Vec<&str> = Vec::new();
    while let Some(curr) = queue.pop_front() {
        topo.push(curr);
        if let Some(neigh) = adjacency.get(curr).cloned() {
            for n in neigh {
                let d = in_degree.entry(n).or_insert(1);
                if *d > 0 {
                    *d -= 1;
                }
                if *d == 0 {
                    queue.push_back(n);
                }
            }
        }
    }
    // Append cycle/orphan nodes.
    let in_topo: HashSet<&str> = topo.iter().copied().collect();
    for n in &code_nodes {
        if !in_topo.contains(n.id.as_str()) {
            topo.push(n.id.as_str());
        }
    }

    let node_map: HashMap<&str, &ua_core::GraphNode> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut steps: Vec<TourStep> = Vec::new();

    if !graph.layers.is_empty() {
        // Group by layer following the topological order.
        let mut node_to_layer: HashMap<&str, &str> = HashMap::new();
        for layer in &graph.layers {
            for nid in &layer.node_ids {
                node_to_layer.insert(nid.as_str(), layer.id.as_str());
            }
        }
        let mut layer_order: Vec<&str> = Vec::new();
        let mut layer_nodes: HashMap<&str, Vec<&str>> = HashMap::new();
        for nid in &topo {
            if let Some(layer_id) = node_to_layer.get(nid) {
                layer_nodes
                    .entry(*layer_id)
                    .or_insert_with(|| {
                        layer_order.push(*layer_id);
                        Vec::new()
                    })
                    .push(*nid);
            }
        }
        let layer_lookup: HashMap<&str, &ua_core::Layer> =
            graph.layers.iter().map(|l| (l.id.as_str(), l)).collect();
        for layer_id in layer_order {
            let nodes = layer_nodes.get(layer_id).cloned().unwrap_or_default();
            if nodes.is_empty() {
                continue;
            }
            let Some(layer) = layer_lookup.get(layer_id) else {
                continue;
            };
            let names: Vec<&str> = nodes
                .iter()
                .filter_map(|id| node_map.get(id).map(|n| n.name.as_str()))
                .collect();
            steps.push(TourStep {
                order: 0,
                title: layer.name.clone(),
                description: format!("{}. Key files: {}.", layer.description, names.join(", ")),
                node_ids: nodes.iter().map(|s| s.to_string()).collect(),
                language_lesson: None,
            });
        }
        // Tail: nodes not in any layer.
        let layered: HashSet<&str> = graph
            .layers
            .iter()
            .flat_map(|l| l.node_ids.iter().map(|s| s.as_str()))
            .collect();
        let unlayered: Vec<&str> = topo
            .iter()
            .copied()
            .filter(|id| !layered.contains(id))
            .collect();
        if !unlayered.is_empty() {
            let names: Vec<&str> = unlayered
                .iter()
                .filter_map(|id| node_map.get(id).map(|n| n.name.as_str()))
                .collect();
            steps.push(TourStep {
                order: 0,
                title: "Supporting Components".into(),
                description: format!("Additional supporting files: {}.", names.join(", ")),
                node_ids: unlayered.iter().map(|s| s.to_string()).collect(),
                language_lesson: None,
            });
        }
    } else {
        // No layers: batch by 3.
        for (i, batch) in topo.chunks(3).enumerate() {
            let summary = batch
                .iter()
                .filter_map(|id| {
                    node_map
                        .get(id)
                        .map(|n| format!("{} ({})", n.name, n.summary))
                })
                .collect::<Vec<_>>()
                .join("; ");
            steps.push(TourStep {
                order: 0,
                title: format!("Step {}: Code Walkthrough", i + 1),
                description: format!("Exploring: {summary}."),
                node_ids: batch.iter().map(|s| s.to_string()).collect(),
                language_lesson: None,
            });
        }
    }

    if !concept_nodes.is_empty() {
        let summary = concept_nodes
            .iter()
            .map(|n| format!("{} ({})", n.name, n.summary))
            .collect::<Vec<_>>()
            .join("; ");
        steps.push(TourStep {
            order: 0,
            title: "Key Concepts".into(),
            description: format!("Important architectural concepts: {summary}."),
            node_ids: concept_nodes.iter().map(|n| n.id.clone()).collect(),
            language_lesson: None,
        });
    }

    for (i, step) in steps.iter_mut().enumerate() {
        step.order = (i + 1) as u32;
    }
    steps
}
