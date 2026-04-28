use ua_core::{
    Complexity, EdgeDirection, EdgeType, GraphEdge, GraphKind, GraphNode, KnowledgeGraph, Layer,
    NodeType, ProjectMeta, TourStep,
};
use ua_persist::Storage;

fn graph() -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "demo".into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "test".into(),
            analyzed_at: "2026-04-27T12:00:00Z".into(),
            git_commit_hash: "abc".into(),
        },
        nodes: vec![
            GraphNode {
                id: "file:src/auth.rs".into(),
                node_type: NodeType::File,
                name: "auth.rs".into(),
                file_path: Some("src/auth.rs".into()),
                line_range: None,
                summary: "user authentication".into(),
                tags: vec!["auth".into(), "security".into()],
                complexity: Complexity::Moderate,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            },
            GraphNode {
                id: "file:src/render.rs".into(),
                node_type: NodeType::File,
                name: "render.rs".into(),
                file_path: Some("src/render.rs".into()),
                line_range: None,
                summary: "frontend rendering".into(),
                tags: vec!["ui".into()],
                complexity: Complexity::Moderate,
                language_notes: None,
                domain_meta: None,
                knowledge_meta: None,
            },
        ],
        edges: vec![GraphEdge {
            source: "file:src/auth.rs".into(),
            target: "file:src/render.rs".into(),
            edge_type: EdgeType::Imports,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 1.0,
        }],
        layers: vec![Layer {
            id: "layer:core".into(),
            name: "Core".into(),
            description: "".into(),
            node_ids: vec!["file:src/auth.rs".into(), "file:src/render.rs".into()],
        }],
        tour: vec![TourStep {
            order: 1,
            title: "start".into(),
            description: "".into(),
            node_ids: vec!["file:src/auth.rs".into()],
            language_lesson: None,
        }],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn embeddings_roundtrip_and_vector_search() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph()).await.unwrap();
    s.ensure_embeddings_table("test-model", 4).await.unwrap();

    let v_auth = vec![1.0_f32, 0.0, 0.0, 0.0];
    let v_render = vec![0.0_f32, 1.0, 0.0, 0.0];
    s.upsert_node_embedding("file:src/auth.rs", "test-model", &v_auth, "h1")
        .await
        .unwrap();
    s.upsert_node_embedding("file:src/render.rs", "test-model", &v_render, "h2")
        .await
        .unwrap();

    assert_eq!(s.embedding_count("test-model").await.unwrap(), 2);
    assert_eq!(
        s.embedding_hash_for("file:src/auth.rs", "test-model")
            .await
            .unwrap()
            .as_deref(),
        Some("h1")
    );

    // Query closer to v_auth — auth row should win.
    let q = vec![0.99_f32, 0.01, 0.0, 0.0];
    let hits = s.vector_scan_top_k("test-model", &q, 2).await.unwrap();
    assert!(!hits.is_empty(), "no hits");
    assert_eq!(hits[0].node_id, "file:src/auth.rs");
    assert!(hits[0].distance < hits.last().unwrap().distance);

    // Forget one, count drops.
    s.forget_embeddings(&["file:src/auth.rs".to_string()])
        .await
        .unwrap();
    assert_eq!(s.embedding_count("test-model").await.unwrap(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn dim_mismatch_reported() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph()).await.unwrap();
    s.ensure_embeddings_table("openai", 1536).await.unwrap();
    let err = s.ensure_embeddings_table("openai", 384).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("mismatch"), "unexpected error: {msg}");
}

#[tokio::test(flavor = "current_thread")]
async fn embedding_dim_for_unknown_model_is_none() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph()).await.unwrap();
    // No `ensure_embeddings_table` call for this model — the metadata
    // row is missing and the helper should report `None` rather than
    // panicking or returning a default dim.
    let dim = s.embedding_dim_for("nonexistent").await.unwrap();
    assert!(dim.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn embedding_hashes_for_returns_only_model_rows() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph()).await.unwrap();
    s.ensure_embeddings_table("a", 4).await.unwrap();
    s.ensure_embeddings_table("b", 4).await.unwrap();
    s.upsert_node_embedding("file:src/auth.rs", "a", &[1.0, 0.0, 0.0, 0.0], "ha")
        .await
        .unwrap();
    s.upsert_node_embedding("file:src/render.rs", "a", &[0.0, 1.0, 0.0, 0.0], "ha2")
        .await
        .unwrap();
    s.upsert_node_embedding("file:src/auth.rs", "b", &[0.0, 0.0, 1.0, 0.0], "hb")
        .await
        .unwrap();

    let map_a = s.embedding_hashes_for("a").await.unwrap();
    assert_eq!(map_a.len(), 2);
    assert_eq!(map_a.get("file:src/auth.rs"), Some(&"ha".to_string()));
    assert_eq!(map_a.get("file:src/render.rs"), Some(&"ha2".to_string()));

    let map_b = s.embedding_hashes_for("b").await.unwrap();
    assert_eq!(map_b.len(), 1);
    assert_eq!(map_b.get("file:src/auth.rs"), Some(&"hb".to_string()));
}

#[tokio::test(flavor = "current_thread")]
async fn reset_drops_only_one_model() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph()).await.unwrap();
    s.ensure_embeddings_table("a", 4).await.unwrap();
    s.ensure_embeddings_table("b", 4).await.unwrap();
    s.upsert_node_embedding("file:src/auth.rs", "a", &[1.0, 0.0, 0.0, 0.0], "h")
        .await
        .unwrap();
    s.upsert_node_embedding("file:src/auth.rs", "b", &[0.0, 1.0, 0.0, 0.0], "h")
        .await
        .unwrap();
    s.reset_embeddings("a").await.unwrap();
    assert_eq!(s.embedding_count("a").await.unwrap(), 0);
    assert_eq!(s.embedding_count("b").await.unwrap(), 1);
    assert!(s.embedding_dim_for("a").await.unwrap().is_none());
    assert_eq!(s.embedding_dim_for("b").await.unwrap(), Some(4));
}
