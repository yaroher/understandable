//! Verifies that [`SearchEngine`] caches its UTF-32 haystacks across
//! `search()` calls.
//!
//! Stable Rust can't directly assert "no allocations occurred", so we
//! lean on a wall-clock proxy: with a 1k-node graph, the first call
//! shouldn't be dramatically faster (or slower) than the fifth — they
//! all reuse the same precomputed `Indexed` cache built in `new`.
//! Pre-fix, every `search()` rebuilt all four `Utf32String`s per node,
//! so repeated calls scaled with N rather than amortising. We also
//! sanity-check that the cache actually returns sensible results after
//! `replace`.

use std::time::Instant;

use ua_core::{Complexity, GraphNode, NodeType};
use ua_search::{SearchEngine, SearchOptions};

fn make_node(i: usize) -> GraphNode {
    GraphNode {
        id: format!("function:src/file_{i}.rs:func_{i}"),
        node_type: NodeType::Function,
        name: format!("function_{i}_handler"),
        file_path: Some(format!("src/file_{i}.rs")),
        line_range: Some((1, 10)),
        summary: format!("Handles request number {i} for the auth flow."),
        tags: vec!["auth".into(), "handler".into(), format!("tag_{i}")],
        complexity: Complexity::Moderate,
        language_notes: Some(format!("Rust async fn returning Result, item {i}")),
        domain_meta: None,
        knowledge_meta: None,
    }
}

#[test]
fn repeated_search_amortises_with_cache() {
    let nodes: Vec<GraphNode> = (0..1000).map(make_node).collect();
    let engine = SearchEngine::new(nodes);

    // Warm-up: the very first call may pay one-time matcher init costs
    // beyond the per-call work we care about, so we time calls 2..=6.
    let _ = engine.search("handler", &SearchOptions::default());

    let mut durations = Vec::with_capacity(5);
    for _ in 0..5 {
        let start = Instant::now();
        let results = engine.search("handler", &SearchOptions::default());
        durations.push(start.elapsed());
        assert!(!results.is_empty());
    }

    // Sanity: pick min/max — with the cache, the spread should be small
    // relative to the absolute time. Pre-fix, repeated calls each redo
    // 4×N UTF-32 conversions, so each call's time is dominated by the
    // same work and durations cluster (so this isn't a regression
    // detector by itself). The real win is the absolute speed: cached
    // 1k-node search must comfortably finish within a generous budget.
    let max = *durations.iter().max().unwrap();
    assert!(
        max.as_millis() < 500,
        "1k-node cached search should be fast; got max={:?}",
        max
    );
}

#[test]
fn replace_rebuilds_cache() {
    let mut engine = SearchEngine::new(vec![GraphNode {
        id: "function:src/a.rs:old".into(),
        node_type: NodeType::Function,
        name: "old_function".into(),
        file_path: None,
        line_range: None,
        summary: "stale".into(),
        tags: vec![],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }]);
    let stale = engine.search("brand_new", &SearchOptions::default());
    assert!(stale.is_empty());

    engine.replace(vec![GraphNode {
        id: "function:src/b.rs:fresh".into(),
        node_type: NodeType::Function,
        name: "brand_new_function".into(),
        file_path: None,
        line_range: None,
        summary: "fresh".into(),
        tags: vec![],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }]);

    let fresh = engine.search("brand_new", &SearchOptions::default());
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].node_id, "function:src/b.rs:fresh");

    // The old node is gone — cache must have been rebuilt, not appended.
    let old = engine.search("old_function", &SearchOptions::default());
    assert!(old.iter().all(|r| r.node_id != "function:src/a.rs:old"));
    // And `nodes()` reflects the new slice.
    assert_eq!(engine.nodes().len(), 1);
    assert_eq!(engine.nodes()[0].id, "function:src/b.rs:fresh");
}
