use ua_core::{Complexity, GraphNode, NodeType};
use ua_search::{
    build_chat_context, format_context_for_prompt, SearchEngine, SearchOptions,
};

fn node(id: &str, name: &str, tags: &[&str], summary: &str) -> GraphNode {
    GraphNode {
        id: id.into(),
        node_type: NodeType::Function,
        name: name.into(),
        file_path: Some("src/auth.rs".into()),
        line_range: Some((1, 10)),
        summary: summary.into(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        complexity: Complexity::Moderate,
        language_notes: None,
        domain_meta: None,
        knowledge_meta: None,
    }
}

#[test]
fn search_finds_node_by_name() {
    let engine = SearchEngine::new(vec![
        node("function:src/a.rs:login", "login", &["auth"], "logs the user in"),
        node("function:src/b.rs:render", "render", &["ui"], "renders the page"),
    ]);
    let results = engine.search("login", &SearchOptions::default());
    assert!(!results.is_empty());
    assert_eq!(results[0].node_id, "function:src/a.rs:login");
    assert!(results[0].score < 0.9);
}

#[test]
fn search_finds_node_by_tag() {
    let engine = SearchEngine::new(vec![
        node("function:src/a.rs:login", "login", &["auth"], ""),
        node("function:src/b.rs:render", "render", &["ui"], ""),
    ]);
    let results = engine.search("auth", &SearchOptions::default());
    assert_eq!(results[0].node_id, "function:src/a.rs:login");
}

#[test]
fn search_respects_type_filter() {
    let mut a = node("function:src/a.rs:foo", "foo", &[], "");
    a.node_type = NodeType::File;
    let mut b = node("function:src/b.rs:foo", "foo", &[], "");
    b.node_type = NodeType::Function;
    let engine = SearchEngine::new(vec![a, b]);
    let opts = SearchOptions {
        types: vec![NodeType::Function],
        limit: None,
    };
    let results = engine.search("foo", &opts);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, "function:src/b.rs:foo");
}

#[test]
fn search_returns_empty_for_blank_query() {
    let engine = SearchEngine::new(vec![node("function:src/a.rs:foo", "foo", &[], "")]);
    assert!(engine.search("", &SearchOptions::default()).is_empty());
    assert!(engine.search("   ", &SearchOptions::default()).is_empty());
}

#[test]
fn chat_context_expands_one_hop() {
    use ua_core::{
        EdgeDirection, EdgeType, GraphEdge, GraphKind, KnowledgeGraph, ProjectMeta,
    };

    let n1 = node("function:src/a.rs:login", "login", &["auth"], "");
    let n2 = node("function:src/b.rs:helper", "helper", &[], "");
    let n3 = node("function:src/c.rs:unrelated", "unrelated", &[], "");
    let g = KnowledgeGraph {
        version: "0.1.0".into(),
        kind: Some(GraphKind::Codebase),
        project: ProjectMeta {
            name: "demo".into(),
            languages: vec![],
            frameworks: vec![],
            description: "".into(),
            analyzed_at: "".into(),
            git_commit_hash: "".into(),
        },
        nodes: vec![n1, n2, n3],
        edges: vec![GraphEdge {
            source: "function:src/a.rs:login".into(),
            target: "function:src/b.rs:helper".into(),
            edge_type: EdgeType::Calls,
            direction: EdgeDirection::Forward,
            description: None,
            weight: 1.0,
        }],
        layers: vec![],
        tour: vec![],
    };
    let ctx = build_chat_context(&g, "auth", 5);
    let ids: Vec<&str> = ctx.relevant_nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"function:src/a.rs:login"));
    assert!(ids.contains(&"function:src/b.rs:helper"));
    assert!(!ids.contains(&"function:src/c.rs:unrelated"));

    let md = format_context_for_prompt(&ctx);
    assert!(md.contains("# Project: demo"));
    assert!(md.contains("login"));
    assert!(md.contains("calls"));
}
