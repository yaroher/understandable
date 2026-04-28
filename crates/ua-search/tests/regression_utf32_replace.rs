//! Regression: `SearchEngine` caches UTF-32 haystacks per node. When
//! `replace` swaps the underlying node set, the cache MUST be rebuilt
//! — earlier code only rotated `nodes` and left the stale `Indexed`
//! vec in place, so searches returned hits for nodes that had been
//! evicted and missed brand-new ones.
//!
//! The companion `tests/cache.rs` covers the basic happy path; this
//! file pins the regression more sharply by walking through several
//! `replace` cycles and asserting that the cache exactly mirrors the
//! current `nodes()` slice.

use ua_core::{Complexity, GraphNode, NodeType};
use ua_search::{SearchEngine, SearchOptions};

fn node(id: &str, name: &str, summary: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        node_type: NodeType::Function,
        name: name.into(),
        file_path: Some(format!("src/{id}.rs")),
        line_range: None,
        summary: summary.into(),
        tags: vec![],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }
}

#[test]
fn replace_invalidates_old_haystack() {
    // Build engine with a node whose name matches "alpha". Replace it
    // with a node that has no "alpha" anywhere — a search for "alpha"
    // must return NO results, proving the old UTF-32 string was
    // discarded and not still ranked.
    let mut engine = SearchEngine::new(vec![node("a", "alpha_handler", "deals with alphas")]);
    let pre = engine.search("alpha", &SearchOptions::default());
    assert!(!pre.is_empty(), "control: pre-replace should match");
    assert!(pre.iter().any(|r| r.node_id == "a"));

    engine.replace(vec![node("b", "completely_unrelated", "no match here")]);

    let post = engine.search("alpha", &SearchOptions::default());
    assert!(
        post.iter().all(|r| r.node_id != "a"),
        "stale node 'a' should not appear after replace, got: {post:?}",
    );
    // Stronger: `alpha` shouldn't match anything in the new set at all.
    assert!(
        post.is_empty(),
        "no node in the new set mentions 'alpha', got: {post:?}",
    );

    // And `nodes()` reflects the new state.
    assert_eq!(engine.nodes().len(), 1);
    assert_eq!(engine.nodes()[0].id, "b");
}

#[test]
fn search_after_replace_returns_new_results() {
    // Symmetric direction: a query that previously matched nothing
    // must hit the new node after `replace`.
    let mut engine = SearchEngine::new(vec![node("a", "alpha_handler", "stale")]);
    let before = engine.search("brand_new_token", &SearchOptions::default());
    assert!(before.is_empty(), "control: nothing matches before replace");

    engine.replace(vec![
        node("b", "brand_new_token_function", "the new shiny one"),
        node("c", "another", "irrelevant"),
    ]);

    let after = engine.search("brand_new_token", &SearchOptions::default());
    assert!(
        after.iter().any(|r| r.node_id == "b"),
        "new node should match its own name, got: {after:?}",
    );

    // And subsequent searches keep working — the cache is stable
    // across multiple post-replace queries.
    let again = engine.search("brand_new_token", &SearchOptions::default());
    assert_eq!(
        after.iter().map(|r| &r.node_id).collect::<Vec<_>>(),
        again.iter().map(|r| &r.node_id).collect::<Vec<_>>(),
    );

    // Multiple replace cycles: each time, the cache should match the
    // current node set exactly.
    engine.replace(vec![node("x", "x_func", "")]);
    let q = engine.search("brand_new_token", &SearchOptions::default());
    assert!(q.is_empty());
    let q2 = engine.search("x_func", &SearchOptions::default());
    assert!(q2.iter().any(|r| r.node_id == "x"));

    engine.replace(vec![]);
    let none = engine.search("anything", &SearchOptions::default());
    assert!(none.is_empty(), "empty replace must yield empty searches");
    assert!(engine.nodes().is_empty());
}
