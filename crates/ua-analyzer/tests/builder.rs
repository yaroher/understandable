use std::collections::HashMap;

use ua_analyzer::{
    apply_llm_layers, detect_layers, generate_heuristic_tour, FileMeta, FileWithAnalysisMeta,
    GraphBuilder, LlmLayerResponse,
};
use ua_core::{Complexity, EdgeType, NodeType, StructuralAnalysis};

#[test]
fn builder_emits_file_function_class_nodes() {
    let mut b = GraphBuilder::new("demo", "abc123");
    b.add_file(
        "src/util.ts",
        FileMeta {
            summary: "utilities".into(),
            tags: vec!["util".into()],
            complexity: Complexity::Simple,
        },
    );

    let analysis = StructuralAnalysis {
        functions: vec![ua_core::FunctionDecl {
            name: "login".into(),
            line_range: (1, 10),
            params: vec!["user".into()],
            return_type: None,
        }],
        classes: vec![ua_core::ClassDecl {
            name: "Auth".into(),
            line_range: (12, 30),
            methods: vec!["verify".into()],
            properties: vec!["token".into()],
        }],
        ..Default::default()
    };

    let mut summaries = HashMap::new();
    summaries.insert("login".into(), "logs the user in".into());
    summaries.insert("Auth".into(), "auth class".into());

    b.add_file_with_analysis(
        "src/auth.ts",
        &analysis,
        FileWithAnalysisMeta {
            file_summary: "auth file".into(),
            tags: vec!["auth".into()],
            complexity: Complexity::Moderate,
            summaries,
        },
    );

    b.add_import_edge("src/auth.ts", "src/util.ts");
    b.add_call_edge("src/auth.ts", "login", "src/util.ts", "checkToken");

    let g = b.build("2026-04-27T00:00:00Z");
    assert_eq!(g.project.name, "demo");
    assert!(g.project.languages.contains(&"typescript".to_string()));

    let func_id = "function:src/auth.ts:login";
    let class_id = "class:src/auth.ts:Auth";
    let file_id = "file:src/auth.ts";
    assert!(g.nodes.iter().any(|n| n.id == func_id && n.node_type == NodeType::Function));
    assert!(g.nodes.iter().any(|n| n.id == class_id && n.node_type == NodeType::Class));
    let file_node = g.nodes.iter().find(|n| n.id == file_id).unwrap();
    assert_eq!(file_node.summary, "auth file");

    // contains edges
    assert!(g.edges.iter().any(|e| e.source == file_id
        && e.target == func_id
        && e.edge_type == EdgeType::Contains));
    // imports edge
    assert!(g.edges.iter().any(|e|
        e.edge_type == EdgeType::Imports
        && e.source == "file:src/auth.ts"
        && e.target == "file:src/util.ts"));
    // calls edge
    assert!(g.edges.iter().any(|e|
        e.edge_type == EdgeType::Calls
        && e.source == "function:src/auth.ts:login"
        && e.target == "function:src/util.ts:checkToken"));
}

#[test]
fn builder_dedups_identical_edges() {
    let mut b = GraphBuilder::new("demo", "");
    b.add_file(
        "a.ts",
        FileMeta {
            summary: "".into(),
            tags: vec![],
            complexity: Complexity::Simple,
        },
    );
    b.add_file(
        "b.ts",
        FileMeta {
            summary: "".into(),
            tags: vec![],
            complexity: Complexity::Simple,
        },
    );
    b.add_import_edge("a.ts", "b.ts");
    b.add_import_edge("a.ts", "b.ts");
    let g = b.build("now");
    let count = g
        .edges
        .iter()
        .filter(|e| e.edge_type == EdgeType::Imports)
        .count();
    assert_eq!(count, 1);
}

/// Pushes the same import edge ten times — only one survives. Exercises
/// the new hash-only `EdgeKeyView` dedup path with repeated probes.
#[test]
fn builder_push_edge_skips_duplicates_repeatedly() {
    let mut b = GraphBuilder::new("demo", "");
    for path in ["a.ts", "b.ts"] {
        b.add_file(
            path,
            FileMeta {
                summary: "".into(),
                tags: vec![],
                complexity: Complexity::Simple,
            },
        );
    }
    for _ in 0..10 {
        b.add_import_edge("a.ts", "b.ts");
    }
    // Two distinct edges with the same endpoints but different types
    // should both survive (dedup is keyed on the EdgeType too).
    b.add_call_edge("a.ts", "x", "b.ts", "y");
    b.add_call_edge("a.ts", "x", "b.ts", "y");
    let g = b.build("now");

    let imports = g
        .edges
        .iter()
        .filter(|e| e.edge_type == EdgeType::Imports)
        .count();
    let calls = g
        .edges
        .iter()
        .filter(|e| e.edge_type == EdgeType::Calls)
        .count();
    assert_eq!(imports, 1);
    assert_eq!(calls, 1);
}

/// `apply_llm_layers` must not panic when the LLM returns two layers
/// with the same `name` — instead the duplicates should merge into one
/// bucket containing every matching file.
#[test]
fn apply_llm_layers_merges_duplicate_names() {
    let mut b = GraphBuilder::new("demo", "");
    for path in [
        "api/users.ts",
        "routes/login.ts",
        "lib/utils.ts",
    ] {
        b.add_file(
            path,
            FileMeta {
                summary: "".into(),
                tags: vec![],
                complexity: Complexity::Simple,
            },
        );
    }
    let g = b.build("now");

    // Two LLM layers share the name "API". The merged layer should
    // contain both `api/users.ts` and `routes/login.ts`.
    let llm = vec![
        LlmLayerResponse {
            name: "API".into(),
            description: "first description".into(),
            file_patterns: vec!["api/".into()],
        },
        LlmLayerResponse {
            name: "API".into(),
            description: "second description (collides)".into(),
            file_patterns: vec!["routes/".into()],
        },
    ];

    // The bug we're guarding against was a panic on `unwrap` —
    // make sure that doesn't happen.
    let layers = apply_llm_layers(&g, &llm);

    let api = layers
        .iter()
        .find(|l| l.name == "API")
        .expect("merged API layer present");
    let mut node_ids = api.node_ids.clone();
    node_ids.sort();
    assert_eq!(
        node_ids,
        vec!["file:api/users.ts", "file:routes/login.ts"]
    );
    // Description comes from the first matching LlmLayerResponse.
    assert_eq!(api.description, "first description");

    // The third file lands in the synthetic "Other" layer.
    let other = layers.iter().find(|l| l.name == "Other").unwrap();
    assert!(other.node_ids.contains(&"file:lib/utils.ts".to_string()));
}

#[test]
fn detect_layers_groups_files_by_directory_pattern() {
    let mut b = GraphBuilder::new("demo", "");
    for path in [
        "src/routes/login.ts",
        "src/services/auth.ts",
        "src/models/user.ts",
        "src/components/Button.tsx",
        "src/utils/format.ts",
        "src/random/orphan.ts",
    ] {
        b.add_file(
            path,
            FileMeta {
                summary: String::new(),
                tags: vec![],
                complexity: Complexity::Simple,
            },
        );
    }
    let g = b.build("now");
    let layers = detect_layers(&g);
    let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains(&"API Layer"), "{names:?}");
    assert!(names.contains(&"Service Layer"));
    assert!(names.contains(&"Data Layer"));
    assert!(names.contains(&"UI Layer"));
    assert!(names.contains(&"Utility Layer"));
    assert!(names.contains(&"Core")); // "src/random/orphan.ts" falls through
}

#[test]
fn heuristic_tour_orders_topologically_in_batches_of_3() {
    let mut b = GraphBuilder::new("demo", "");
    for i in 0..5 {
        b.add_file(
            &format!("src/f{}.ts", i),
            FileMeta {
                summary: format!("file {i}"),
                tags: vec![],
                complexity: Complexity::Simple,
            },
        );
    }
    // Make a chain: f0 -> f1 -> f2 -> f3 -> f4
    for i in 0..4 {
        b.add_import_edge(&format!("src/f{}.ts", i), &format!("src/f{}.ts", i + 1));
    }
    let g = b.build("now");
    let tour = generate_heuristic_tour(&g);
    assert!(!tour.is_empty());
    // 5 nodes batched by 3 → ceil(5/3)=2 walkthrough steps.
    assert_eq!(tour.len(), 2);
    assert_eq!(tour[0].order, 1);
    assert_eq!(tour[1].order, 2);
    // Topological start should be f0.
    assert!(tour[0].node_ids[0].ends_with("f0.ts"));
}

#[test]
fn heuristic_tour_uses_layers_when_present() {
    let mut b = GraphBuilder::new("demo", "");
    b.add_file(
        "src/routes/r.ts",
        FileMeta {
            summary: "r".into(),
            tags: vec![],
            complexity: Complexity::Simple,
        },
    );
    b.add_file(
        "src/services/s.ts",
        FileMeta {
            summary: "s".into(),
            tags: vec![],
            complexity: Complexity::Simple,
        },
    );
    b.add_import_edge("src/routes/r.ts", "src/services/s.ts");
    let mut g = b.build("now");
    g.layers = detect_layers(&g);
    let tour = generate_heuristic_tour(&g);
    let titles: Vec<&str> = tour.iter().map(|t| t.title.as_str()).collect();
    assert!(titles.contains(&"API Layer"));
    assert!(titles.contains(&"Service Layer"));
}

