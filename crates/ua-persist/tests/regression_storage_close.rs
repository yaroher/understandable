//! Regression: `Storage::close` was removed after a double-flush bug.
//! Persistence now happens exclusively through `save` / `save_kind`,
//! and `Drop` is the implicit no-op cleanup. This pins:
//!
//!   * dropping `Storage` after a successful `save` is clean — no
//!     `.tmp` files left in the storage dir, archive intact.
//!   * dropping without `save` does NOT corrupt the on-disk archive
//!     from a previous session — it simply doesn't write anything new.
//!
//! These tests would have caught the double-flush regression, where a
//! removed `close` left a stray `.tmp` after the explicit `save`.

use ua_core::{
    Complexity, GraphKind, GraphNode, KnowledgeGraph, NodeType, ProjectMeta,
};
use ua_persist::{ProjectLayout, Storage};

fn tiny_graph(name: &str) -> KnowledgeGraph {
    KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: name.into(),
            languages: vec!["rust".into()],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "2026-04-28T00:00:00Z".into(),
            git_commit_hash: "abc".into(),
        },
        nodes: vec![GraphNode {
            id: format!("file:{name}.rs"),
            node_type: NodeType::File,
            name: format!("{name}.rs"),
            file_path: Some(format!("{name}.rs")),
            line_range: None,
            summary: "".into(),
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

fn tmp_path_for(zst: &std::path::Path) -> std::path::PathBuf {
    let mut s = zst.as_os_str().to_os_string();
    s.push(".tmp");
    std::path::PathBuf::from(s)
}

#[tokio::test(flavor = "current_thread")]
async fn drop_after_save_is_clean() {
    // Save once, then let `Storage` drop. The archive must exist, no
    // stale `.tmp` leftover, and the bytes must be readable on the next
    // open.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    let archive_bytes = {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&tiny_graph("alpha")).await.unwrap();
        s.save(&layout).await.unwrap();
        let bytes = std::fs::read(layout.graph_archive()).unwrap();
        // Drop happens at the end of this scope.
        bytes
    };
    assert!(!archive_bytes.is_empty(), "archive must have content");

    let zst = layout.graph_archive();
    assert!(zst.exists(), "archive missing after drop");
    let tmp = tmp_path_for(&zst);
    assert!(
        !tmp.exists(),
        ".tmp must not be left behind after a clean drop",
    );

    // Bytes on disk are stable across the drop — they match what we
    // captured before the implicit Drop ran. (No "second flush" wrote
    // a different blob.)
    let after_drop = std::fs::read(&zst).unwrap();
    assert_eq!(after_drop, archive_bytes, "drop must not rewrite archive");

    // And reopening must succeed.
    let s2 = Storage::open(&layout).await.unwrap();
    let g = s2.load_graph().await.unwrap();
    assert_eq!(g.project.name, "alpha");
}

#[tokio::test(flavor = "current_thread")]
async fn drop_without_save_does_not_corrupt_archive() {
    // Set up a real archive, then re-open and drop without saving. The
    // existing archive must come through byte-identical.
    let project = tempfile::tempdir().unwrap();
    let layout = ProjectLayout::under(project.path());

    {
        let s = Storage::open(&layout).await.unwrap();
        s.save_graph(&tiny_graph("baseline")).await.unwrap();
        s.save(&layout).await.unwrap();
    }
    let zst = layout.graph_archive();
    let baseline = std::fs::read(&zst).unwrap();

    {
        // Open, mutate in memory, drop without save — the on-disk
        // archive must NOT change.
        let s = Storage::open(&layout).await.unwrap();
        s.cache_llm_output("file:baseline.rs", "p", "f", "should-not-persist")
            .await
            .unwrap();
        // Drop here — no save called.
    }

    let after_drop = std::fs::read(&zst).unwrap();
    assert_eq!(
        after_drop, baseline,
        "drop-without-save must leave the archive bit-identical",
    );

    // No stray .tmp either.
    let tmp = tmp_path_for(&zst);
    assert!(!tmp.exists());

    // And reopen still loads the original graph.
    let s = Storage::open(&layout).await.unwrap();
    let g = s.load_graph().await.unwrap();
    assert_eq!(g.project.name, "baseline");
    // The transient cache entry from the dropped session is NOT here.
    let miss = s
        .llm_output_for("file:baseline.rs", "p", "f")
        .await
        .unwrap();
    assert!(
        miss.is_none(),
        "drop-without-save must not have flushed the in-memory cache",
    );
}
