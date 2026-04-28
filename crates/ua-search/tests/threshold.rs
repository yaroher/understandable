//! Hardens the empty-query short-circuit and the relevance-threshold
//! cutoff in [`SearchEngine::search`]. The existing `search.rs` test
//! suite already exercises happy-path ranking; these cases protect the
//! two behaviours that quietly break when somebody refactors the
//! scoring loop.

use ua_core::{Complexity, GraphNode, NodeType};
use ua_search::{SearchEngine, SearchOptions};

fn node(id: &str, name: &str, summary: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        node_type: NodeType::Function,
        name: name.into(),
        file_path: Some("src/lib.rs".into()),
        line_range: Some((1, 10)),
        summary: summary.into(),
        tags: vec![],
        complexity: Complexity::Simple,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }
}

#[test]
fn empty_query_returns_nothing() {
    let engine = SearchEngine::new(vec![
        node("function:src/a.rs:foo", "foo", "does foo"),
        node("function:src/b.rs:bar", "bar", "does bar"),
    ]);
    assert!(engine.search("", &SearchOptions::default()).is_empty());
    assert!(engine.search("   ", &SearchOptions::default()).is_empty());
    assert!(engine.search("\t\n", &SearchOptions::default()).is_empty());
}

#[test]
fn threshold_cutoff_filters_irrelevant_matches() {
    // The query bears no resemblance to any node — every fuzzy score
    // should land above the cutoff and the engine should return zero
    // hits rather than ranking unrelated nodes.
    let engine = SearchEngine::new(vec![
        node("function:src/a.rs:login", "login", "authenticate"),
        node("function:src/b.rs:render", "render", "draws ui"),
    ]);
    let results = engine.search("zzzqxv", &SearchOptions::default());
    assert!(
        results.is_empty(),
        "expected no hits for unrelated query, got {results:?}"
    );
}

#[test]
fn good_match_scores_below_one() {
    // Sanity check: a sensible query should bring scores well below the
    // 0.95 cutoff so we know the threshold isn't simply rejecting
    // everything.
    let engine = SearchEngine::new(vec![node(
        "function:src/a.rs:authenticate",
        "authenticate",
        "log a user in",
    )]);
    let results = engine.search("authenticate", &SearchOptions::default());
    assert_eq!(results.len(), 1);
    assert!(
        results[0].score < 0.9,
        "expected strong match to score below threshold cutoff, got {}",
        results[0].score
    );
}
