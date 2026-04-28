//! Sanity: `vector_top_k` returns sensible orderings on a small corpus
//! and rejects dim mismatches via two paths (registered meta row and
//! the orphan-row cross-check).

use ua_core::Error;
use ua_core::{Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta};
use ua_persist::{ProjectLayout, Storage};

fn tiny_corpus() -> KnowledgeGraph {
    let mk_node = |id: &str, name: &str| GraphNode {
        id: id.into(),
        node_type: NodeType::File,
        name: name.into(),
        file_path: Some(format!("{name}.rs")),
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
            name: "v".into(),
            languages: vec![],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "".into(),
            git_commit_hash: "".into(),
        },
        nodes: vec![
            mk_node("n:north", "north"),
            mk_node("n:east", "east"),
            mk_node("n:south", "south"),
            mk_node("n:west", "west"),
        ],
        edges: vec![],
        layers: vec![],
        tour: vec![],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn vector_top_k_orders_by_cosine_distance() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&tiny_corpus()).await.unwrap();
    s.ensure_embeddings_table("m", 4).await.unwrap();

    // Each node lives on a distinct axis so cosine distance is
    // unambiguous.
    let rows: Vec<(&str, &str, &[f32])> = vec![
        ("n:north", "h-n", &[1.0, 0.0, 0.0, 0.0]),
        ("n:east", "h-e", &[0.0, 1.0, 0.0, 0.0]),
        ("n:south", "h-s", &[0.0, 0.0, 1.0, 0.0]),
        ("n:west", "h-w", &[0.0, 0.0, 0.0, 1.0]),
    ];
    s.upsert_node_embeddings_batch("m", &rows).await.unwrap();
    assert_eq!(s.embedding_count("m").await.unwrap(), 4);

    // Query right next to "east".
    let q = vec![0.05_f32, 0.99, 0.05, 0.05];
    let hits = s.vector_top_k("m", &q, 4).await.unwrap();
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].node_id, "n:east", "got: {hits:?}");
    // Distances must be monotonically non-decreasing.
    for w in hits.windows(2) {
        assert!(
            w[0].distance <= w[1].distance,
            "distances out of order: {hits:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dim_mismatch_via_meta_table() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&tiny_corpus()).await.unwrap();
    s.ensure_embeddings_table("model_x", 16).await.unwrap();
    let err = s.ensure_embeddings_table("model_x", 32).await.unwrap_err();
    match err {
        Error::EmbeddingDimMismatch { model, stored, new } => {
            assert_eq!(model, "model_x");
            assert_eq!(stored, 16);
            assert_eq!(new, 32);
        }
        other => panic!("expected dim mismatch, got: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dim_mismatch_via_orphan_rows() {
    // Insert a real row at dim=8 with model A, then ask for model B
    // at dim=12 without a `reset`. The orphan-row cross-check (i.e.
    // "any persisted vector at the wrong dim") must surface a loud
    // dim mismatch — same UX as the old `F32_BLOB(8)` physical-column
    // failure path.
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&tiny_corpus()).await.unwrap();
    s.ensure_embeddings_table("model_a", 8).await.unwrap();
    let v_a = vec![0.1f32; 8];
    s.upsert_node_embedding("n:north", "model_a", &v_a, "h-a")
        .await
        .unwrap();

    let err = s.ensure_embeddings_table("model_b", 12).await.unwrap_err();
    match err {
        Error::EmbeddingDimMismatch { stored, new, .. } => {
            assert_eq!(stored, 8);
            assert_eq!(new, 12);
        }
        other => panic!("expected dim mismatch, got: {other:?}"),
    }
}

/// Save a graph + embeddings to a real `tar.zst` archive, reopen it,
/// and confirm `vector_top_k` works without first paying a rebuild —
/// i.e. the cold-open view path is exercised. We can't peek at
/// `IndexState` directly (it's private), but we *can* assert that the
/// archive contains the new `vectors.usearch` entry and that the
/// reopened storage yields the same top-1 hit as the source store.
#[tokio::test(flavor = "current_thread")]
async fn cold_open_uses_view() {
    let dir = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(dir.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&tiny_corpus()).await.unwrap();
        s.ensure_embeddings_table("m", 4).await.unwrap();
        let rows: Vec<(&str, &str, &[f32])> = vec![
            ("n:north", "h-n", &[1.0, 0.0, 0.0, 0.0]),
            ("n:east", "h-e", &[0.0, 1.0, 0.0, 0.0]),
            ("n:south", "h-s", &[0.0, 0.0, 1.0, 0.0]),
            ("n:west", "h-w", &[0.0, 0.0, 0.0, 1.0]),
        ];
        s.upsert_node_embeddings_batch("m", &rows).await.unwrap();
        s.save(&layout).await.unwrap();
    }

    // Confirm the new single-file usearch dump made it into the
    // archive — that's what backs the mmap'd `Index::view`.
    let archive_path = layout.graph_archive();
    let bytes = std::fs::read(&archive_path).unwrap();
    let dec = zstd::stream::read::Decoder::new(&bytes[..]).unwrap();
    let mut tar = tar::Archive::new(dec);
    let mut found = false;
    for e in tar.entries().unwrap() {
        let e = e.unwrap();
        let p = e.path().unwrap().to_string_lossy().into_owned();
        if p == "vectors.usearch" {
            found = true;
        }
        // Legacy entries must not be there.
        assert_ne!(p, "vectors.hnsw.graph");
        assert_ne!(p, "vectors.hnsw.data");
    }
    assert!(found, "archive must carry vectors.usearch");

    // Reopen and query — this hits the cold-open view path.
    let s = Storage::open(&layout).await.unwrap();
    let q = vec![0.05_f32, 0.99, 0.05, 0.05];
    let hits = s.vector_top_k("m", &q, 4).await.unwrap();
    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].node_id, "n:east", "got: {hits:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn batch_upsert_is_idempotent_on_text_hash_check() {
    let s = Storage::open_fresh().await.unwrap();
    s.save_graph(&tiny_corpus()).await.unwrap();
    s.ensure_embeddings_table("m", 4).await.unwrap();

    let rows: Vec<(&str, &str, &[f32])> = vec![("n:north", "v1", &[1.0, 0.0, 0.0, 0.0])];
    s.upsert_node_embeddings_batch("m", &rows).await.unwrap();
    s.upsert_node_embeddings_batch("m", &rows).await.unwrap();
    assert_eq!(s.embedding_count("m").await.unwrap(), 1);

    // Confirm the hash sticks.
    assert_eq!(
        s.embedding_hash_for("n:north", "m")
            .await
            .unwrap()
            .as_deref(),
        Some("v1")
    );
}
