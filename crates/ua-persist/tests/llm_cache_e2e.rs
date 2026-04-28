//! End-to-end tests for the per-file LLM output cache.
//!
//! The cache key is `(node_id, prompt_hash)`; the entry carries
//! `file_hash` so the lookup turns into a miss the moment the file
//! changes on disk. These tests pin both the in-memory semantics and
//! the round-trip through `tar.zst`.

use ua_core::{Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta};
use ua_persist::{ProjectLayout, Storage};

fn empty_graph() -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "llm-cache".into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "2026-04-28T00:00:00Z".into(),
            git_commit_hash: "deadbeef".into(),
        },
        nodes: vec![GraphNode {
            id: "file:src/auth.rs".into(),
            node_type: NodeType::File,
            name: "auth.rs".into(),
            file_path: Some("src/auth.rs".into()),
            line_range: None,
            summary: "auth".into(),
            tags: vec![],
            complexity: Complexity::Simple,
            language_notes: None,
            domain_meta: None,
            knowledge_meta: None,
        }],
        edges: vec![],
        layers: vec![],
        tour: vec![],
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cache_hit_returns_cached_response() {
    let s = Storage::open_fresh().await.unwrap();
    s.cache_llm_output(
        "file:src/auth.rs",
        "prompt-h1",
        "file-h1",
        "{\"summary\":\"ok\"}",
    )
    .await
    .unwrap();

    let hit = s
        .llm_output_for("file:src/auth.rs", "prompt-h1", "file-h1")
        .await
        .unwrap();
    assert_eq!(hit.as_deref(), Some("{\"summary\":\"ok\"}"));
}

#[tokio::test(flavor = "current_thread")]
async fn cache_miss_when_file_hash_changes() {
    let s = Storage::open_fresh().await.unwrap();
    s.cache_llm_output("file:src/auth.rs", "prompt-h1", "file-h1", "stale")
        .await
        .unwrap();

    // Same node + same prompt, but the file body has changed: must miss
    // (so the caller knows to re-run the LLM).
    let miss = s
        .llm_output_for("file:src/auth.rs", "prompt-h1", "file-h2-new")
        .await
        .unwrap();
    assert!(miss.is_none(), "stale entry must miss on file_hash change");
}

#[tokio::test(flavor = "current_thread")]
async fn cache_miss_when_prompt_hash_changes() {
    let s = Storage::open_fresh().await.unwrap();
    s.cache_llm_output("file:src/auth.rs", "prompt-h1", "file-h1", "first")
        .await
        .unwrap();

    // Same node + same file body but a different prompt hash → miss.
    let miss = s
        .llm_output_for("file:src/auth.rs", "prompt-h2", "file-h1")
        .await
        .unwrap();
    assert!(miss.is_none(), "different prompt_hash must miss");
}

#[tokio::test(flavor = "current_thread")]
async fn cache_round_trips_through_archive() {
    // Save a populated cache to a tar.zst, reopen, verify hits survive.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        // We need *some* graph payload so save() has a graph_msgpack
        // entry (cache survives independently, but a fully empty save
        // is closer to a no-op corner case — populate the graph).
        s.save_graph_for(&empty_graph(), &layout).await.unwrap();
        s.cache_llm_output("file:src/auth.rs", "p", "f", "the-response")
            .await
            .unwrap();
        s.cache_llm_output("file:src/render.rs", "p", "f2", "render-resp")
            .await
            .unwrap();
        s.save(&layout).await.unwrap();
    }

    // Cold-open: cache must come back intact.
    let s = Storage::open(&layout).await.unwrap();
    let hit = s
        .llm_output_for("file:src/auth.rs", "p", "f")
        .await
        .unwrap();
    assert_eq!(hit.as_deref(), Some("the-response"));

    let hit2 = s
        .llm_output_for("file:src/render.rs", "p", "f2")
        .await
        .unwrap();
    assert_eq!(hit2.as_deref(), Some("render-resp"));

    // And a wrong-hash lookup still misses after the round-trip.
    let miss = s
        .llm_output_for("file:src/auth.rs", "p", "different")
        .await
        .unwrap();
    assert!(miss.is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn forget_llm_outputs_drops_entries() {
    let s = Storage::open_fresh().await.unwrap();
    s.cache_llm_output("file:src/keep.rs", "p", "f", "keep-me")
        .await
        .unwrap();
    s.cache_llm_output("file:src/gone.rs", "p", "f", "drop-me")
        .await
        .unwrap();

    // forget the deleted file's entries — the surviving entry stays.
    s.forget_llm_outputs(&["file:src/gone.rs".to_string()])
        .await
        .unwrap();

    let kept = s
        .llm_output_for("file:src/keep.rs", "p", "f")
        .await
        .unwrap();
    assert_eq!(kept.as_deref(), Some("keep-me"));

    let gone = s
        .llm_output_for("file:src/gone.rs", "p", "f")
        .await
        .unwrap();
    assert!(gone.is_none(), "gone.rs entry must be dropped");
}
