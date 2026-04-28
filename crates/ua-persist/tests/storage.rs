use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphNode, KnowledgeGraph, Layer, NodeType,
    ProjectMeta, TourStep,
};
use ua_persist::{blake3_string, Fingerprint, ProjectLayout, Storage};

fn sample_graph() -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(ua_core::GraphKind::Codebase),
        project: ProjectMeta {
            name: "demo".into(),
            languages: vec!["rust".into(), "typescript".into()],
            frameworks: vec![],
            description: "test".into(),
            analyzed_at: "2026-04-27T12:00:00Z".into(),
            git_commit_hash: "abcdef".into(),
        },
        nodes: vec![
            GraphNode {
                id: "file:src/main.rs".into(),
                node_type: NodeType::File,
                name: "main.rs".into(),
                file_path: Some("src/main.rs".into()),
                line_range: None,
                summary: "entry point".into(),
                tags: vec!["entry".into(), "binary".into()],
                complexity: Complexity::Simple,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            },
            GraphNode {
                id: "function:src/main.rs:main".into(),
                node_type: NodeType::Function,
                name: "main".into(),
                file_path: Some("src/main.rs".into()),
                line_range: Some((1, 10)),
                summary: "starts the server".into(),
                tags: vec!["entrypoint".into()],
                complexity: Complexity::Moderate,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            },
        ],
        edges: vec![GraphEdge {
            source: "file:src/main.rs".into(),
            target: "function:src/main.rs:main".into(),
            edge_type: EdgeType::Contains,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 1.0,
        }],
        layers: vec![Layer {
            id: "layer:core".into(),
            name: "Core".into(),
            description: "Core files".into(),
            node_ids: vec!["file:src/main.rs".into()],
        }],
        tour: vec![TourStep {
            order: 1,
            title: "Start here".into(),
            description: "the entrypoint".into(),
            node_ids: vec!["file:src/main.rs".into()],
            language_lesson: None,
        }],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fresh_storage_save_load_roundtrip() {
    let s = Storage::open_fresh().await.unwrap();
    let g = sample_graph();
    s.save_graph(&g).await.unwrap();
    let g2 = s.load_graph().await.unwrap();
    assert_eq!(g.nodes, g2.nodes);
    assert_eq!(g.edges, g2.edges);
    assert_eq!(g.layers, g2.layers);
    assert_eq!(g.tour, g2.tour);
    assert_eq!(g.project, g2.project);
    assert_eq!(g.kind, g2.kind);
}

#[tokio::test(flavor = "current_thread")]
async fn save_to_layout_then_reopen() {
    let project_dir = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project_dir.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&sample_graph()).await.unwrap();
        s.save(&layout).await.unwrap();
    }

    let zst = layout.graph_archive();
    assert!(zst.exists());
    let raw_bytes = std::fs::metadata(&zst).unwrap().len();
    assert!(raw_bytes > 0);

    let s = Storage::open(&layout).await.unwrap();
    let g2 = s.load_graph().await.unwrap();
    let g = sample_graph();
    assert_eq!(g.nodes, g2.nodes);
    assert_eq!(g.edges, g2.edges);
    assert_eq!(g.project, g2.project);
}

#[tokio::test(flavor = "current_thread")]
async fn fts_search_finds_node() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&sample_graph()).await.unwrap();
    let hits = s.search_nodes("entry", 5).await.unwrap();
    assert!(
        hits.contains(&"file:src/main.rs".to_string()),
        "got {hits:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn outgoing_edges_query() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&sample_graph()).await.unwrap();
    let out = s.outgoing_edges("file:src/main.rs").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "function:src/main.rs:main");
    assert_eq!(out[0].1, "contains");
}

#[tokio::test(flavor = "current_thread")]
async fn fingerprint_roundtrip() {
    let s = Storage::open_fresh().await.unwrap();
    let prints = vec![
        Fingerprint {
            path: "src/a.rs".into(),
            hash: blake3_string(b"a"),
            modified_at: Some(1_000_000),
            structural_hash: None,
        },
        Fingerprint {
            path: "src/b.rs".into(),
            hash: blake3_string(b"b"),
            modified_at: None,
            structural_hash: Some("deadbeef".into()),
        },
    ];
    s.write_fingerprints(&prints).await.unwrap();
    let mut got = s.read_fingerprints().await.unwrap();
    got.sort_by(|a, b| a.path.cmp(&b.path));
    let mut want = prints.clone();
    want.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(got, want);
}

#[tokio::test(flavor = "current_thread")]
async fn save_replaces_existing_graph() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&sample_graph()).await.unwrap();
    let mut g2 = sample_graph();
    g2.nodes.truncate(1);
    g2.edges.clear();
    s.save_graph(&g2).await.unwrap();
    let loaded = s.load_graph().await.unwrap();
    assert_eq!(loaded.nodes.len(), 1);
    assert_eq!(loaded.edges.len(), 0);
}
