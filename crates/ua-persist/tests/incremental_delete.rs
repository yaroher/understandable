//! Incremental delete pathway — `forget_embeddings` and
//! `forget_llm_outputs` drop only the named nodes, leaving everything
//! else intact. Mirrors what the `analyze --incremental` pipeline does
//! when files vanish from the working tree.

use ua_core::{
    Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
};
use ua_persist::Storage;

fn graph_with_three_files() -> KnowledgeGraph {
    let mk = |id: &str, name: &str, path: &str| GraphNode {
        id: id.into(),
        node_type: NodeType::File,
        name: name.into(),
        file_path: Some(path.into()),
        line_range: None,
        summary: "".into(),
        tags: vec![],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    };
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "incr".into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "2026-04-28T00:00:00Z".into(),
            git_commit_hash: "abc".into(),
        },
        nodes: vec![
            mk("file:src/keep.rs", "keep.rs", "src/keep.rs"),
            mk("file:src/drop.rs", "drop.rs", "src/drop.rs"),
            mk("file:src/also_keep.rs", "also_keep.rs", "src/also_keep.rs"),
        ],
        edges: vec![],
        layers: vec![],
        tour: vec![],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn forget_embeddings_drops_only_named_nodes() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph_with_three_files()).await.unwrap();
    s.ensure_embeddings_table("test-model", 4).await.unwrap();

    s.upsert_node_embedding("file:src/keep.rs", "test-model", &[1.0, 0.0, 0.0, 0.0], "h-keep")
        .await
        .unwrap();
    s.upsert_node_embedding("file:src/drop.rs", "test-model", &[0.0, 1.0, 0.0, 0.0], "h-drop")
        .await
        .unwrap();
    s.upsert_node_embedding(
        "file:src/also_keep.rs",
        "test-model",
        &[0.0, 0.0, 1.0, 0.0],
        "h-also",
    )
    .await
    .unwrap();
    assert_eq!(s.embedding_count("test-model").await.unwrap(), 3);

    s.forget_embeddings(&["file:src/drop.rs".to_string()])
        .await
        .unwrap();

    assert_eq!(s.embedding_count("test-model").await.unwrap(), 2);
    let hashes = s.embedding_hashes_for("test-model").await.unwrap();
    assert!(hashes.contains_key("file:src/keep.rs"));
    assert!(hashes.contains_key("file:src/also_keep.rs"));
    assert!(
        !hashes.contains_key("file:src/drop.rs"),
        "dropped row must be gone"
    );
    // And vector_scan still works on the survivors.
    let hits = s
        .vector_scan_top_k("test-model", &[1.0, 0.0, 0.0, 0.0], 5)
        .await
        .unwrap();
    let ids: Vec<_> = hits.iter().map(|h| h.node_id.as_str()).collect();
    assert!(ids.contains(&"file:src/keep.rs"));
    assert!(!ids.contains(&"file:src/drop.rs"));
}

#[tokio::test(flavor = "current_thread")]
async fn forget_embeddings_empty_input_is_noop() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&graph_with_three_files()).await.unwrap();
    s.ensure_embeddings_table("m", 4).await.unwrap();
    s.upsert_node_embedding("file:src/keep.rs", "m", &[1.0, 0.0, 0.0, 0.0], "h")
        .await
        .unwrap();

    // Empty slice: nothing should change.
    s.forget_embeddings(&[]).await.unwrap();

    assert_eq!(s.embedding_count("m").await.unwrap(), 1);
    assert_eq!(
        s.embedding_hash_for("file:src/keep.rs", "m").await.unwrap(),
        Some("h".to_string())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn forget_llm_outputs_after_file_delete() {
    // Mirrors the analyze pipeline: a file vanished from the working
    // tree, so its cache entries should be dropped while every other
    // node's entries survive.
    let s = Storage::open_fresh().await.unwrap();
    // Three files, two with cached LLM responses for two distinct
    // prompts — proves the drop is by node_id, not by prompt.
    s.cache_llm_output("file:src/keep.rs", "p1", "f1", "k1")
        .await
        .unwrap();
    s.cache_llm_output("file:src/keep.rs", "p2", "f1", "k2")
        .await
        .unwrap();
    s.cache_llm_output("file:src/gone.rs", "p1", "f1", "g1")
        .await
        .unwrap();
    s.cache_llm_output("file:src/gone.rs", "p2", "f1", "g2")
        .await
        .unwrap();

    s.forget_llm_outputs(&["file:src/gone.rs".to_string()])
        .await
        .unwrap();

    // keep.rs still has both prompts cached.
    assert_eq!(
        s.llm_output_for("file:src/keep.rs", "p1", "f1")
            .await
            .unwrap()
            .as_deref(),
        Some("k1"),
    );
    assert_eq!(
        s.llm_output_for("file:src/keep.rs", "p2", "f1")
            .await
            .unwrap()
            .as_deref(),
        Some("k2"),
    );
    // gone.rs entries are gone.
    assert!(s
        .llm_output_for("file:src/gone.rs", "p1", "f1")
        .await
        .unwrap()
        .is_none());
    assert!(s
        .llm_output_for("file:src/gone.rs", "p2", "f1")
        .await
        .unwrap()
        .is_none());

    // Empty drop list is also a no-op (early return).
    s.forget_llm_outputs(&[]).await.unwrap();
    assert_eq!(
        s.llm_output_for("file:src/keep.rs", "p1", "f1")
            .await
            .unwrap()
            .as_deref(),
        Some("k1"),
    );
}
