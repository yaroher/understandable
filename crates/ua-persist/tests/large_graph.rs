//! Round-trip a non-trivial graph (50 nodes / 200 edges) through
//! `save_graph` + `load_graph` and verify equality. Smoke-checks the
//! IndraDB MessagePack dump path on something denser than the tiny
//! samples used elsewhere.

use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, Layer,
    NodeType, ProjectMeta, TourStep,
};
use ua_persist::{ProjectLayout, Storage};

fn build_graph() -> KnowledgeGraph {
    let mut nodes = Vec::with_capacity(50);
    for i in 0..50 {
        nodes.push(GraphNode {
            id: format!("file:src/m{i}.rs"),
            node_type: NodeType::File,
            name: format!("m{i}.rs"),
            file_path: Some(format!("src/m{i}.rs")),
            line_range: Some((1, (i as u32) + 10)),
            summary: format!("module {i} summary"),
            tags: vec![format!("tag-{i}"), "module".into()],
            complexity: if i % 3 == 0 {
                Complexity::Simple
            } else if i % 3 == 1 {
                Complexity::Moderate
            } else {
                Complexity::Complex
            },
            language_notes: Some(format!("rust {i}")),
            domain_meta: None,
            knowledge_meta: None,
        });
    }
    // Build unique (source, target, type) triples: pair every node
    // with its 4 deterministic neighbours = 200 edges total. Avoiding
    // duplicates is load-bearing because IndraDB stores edges by
    // (outbound, type, inbound) — inserting the same triple twice is
    // a no-op, which would make round-trip equality flake.
    let mut edges = Vec::with_capacity(200);
    for src in 0..50 {
        for off in 1..=4 {
            let dst = (src + off * 7 + 3) % 50;
            if src == dst {
                continue;
            }
            edges.push(GraphEdge {
                source: format!("file:src/m{src}.rs"),
                target: format!("file:src/m{dst}.rs"),
                edge_type: if off % 2 == 0 {
                    EdgeType::Imports
                } else {
                    EdgeType::Calls
                },
                direction: EdgeDirection::Forward,
                description: Some(format!("edge {src}-{off}")),
                weight: 0.5,
            });
        }
    }
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "stress".into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "200 edges".into(),
            analyzed_at: "2026-04-28T12:00:00Z".into(),
            git_commit_hash: "stress01".into(),
        },
        nodes,
        edges,
        layers: vec![Layer {
            id: "layer:all".into(),
            name: "All".into(),
            description: "every module".into(),
            node_ids: (0..50).map(|i| format!("file:src/m{i}.rs")).collect(),
        }],
        tour: vec![TourStep {
            order: 1,
            title: "tour".into(),
            description: "intro".into(),
            node_ids: vec!["file:src/m0.rs".into()],
            language_lesson: None,
        }],
    }
}

fn sort_graph(g: &mut KnowledgeGraph) {
    g.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    g.edges.sort_by(|a, b| {
        (
            a.source.as_str(),
            a.target.as_str(),
            edge_type_str(a.edge_type),
        )
            .cmp(&(
                b.source.as_str(),
                b.target.as_str(),
                edge_type_str(b.edge_type),
            ))
    });
}

fn edge_type_str(t: EdgeType) -> &'static str {
    match t {
        EdgeType::Imports => "imports",
        EdgeType::Calls => "calls",
        _ => "other",
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fifty_nodes_two_hundred_edges_roundtrip_in_memory() {
    let s = Storage::open_fresh().await.unwrap();
    let g = build_graph();
    s.save_graph(&g).await.unwrap();
    let mut loaded = s.load_graph().await.unwrap();
    let mut expected = g.clone();
    sort_graph(&mut loaded);
    sort_graph(&mut expected);
    assert_eq!(loaded.nodes, expected.nodes);
    assert_eq!(loaded.edges, expected.edges);
    assert_eq!(loaded.layers, expected.layers);
    assert_eq!(loaded.tour, expected.tour);
    assert_eq!(loaded.project, expected.project);
}

#[tokio::test(flavor = "current_thread")]
async fn fifty_nodes_two_hundred_edges_roundtrip_through_archive() {
    let dir = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(dir.path());
    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&build_graph()).await.unwrap();
        s.save(&layout).await.unwrap();
    }
    let archive = layout.graph_archive();
    assert!(archive.exists());
    let bytes = std::fs::metadata(&archive).unwrap().len();
    assert!(bytes > 0);

    let s = Storage::open(&layout).await.unwrap();
    let mut loaded = s.load_graph().await.unwrap();
    let mut expected = build_graph();
    sort_graph(&mut loaded);
    sort_graph(&mut expected);
    assert_eq!(loaded.nodes.len(), 50);
    assert_eq!(loaded.edges, expected.edges);
    assert_eq!(loaded.project, expected.project);
}
