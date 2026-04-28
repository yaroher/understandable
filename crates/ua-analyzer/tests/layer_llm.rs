//! Coverage for `parse_layer_detection_response` + `apply_llm_layers`.
//!
//! The two functions form the LLM-driven half of layer detection; the
//! deterministic `detect_layers` is exercised in `builder.rs`. These
//! tests guard the parser's fence-stripping / garbage-tolerance and
//! verify that every file node is assigned to exactly one layer with
//! unmatched files landing in a synthetic "Other" bucket.

use ua_analyzer::{
    apply_llm_layers, parse_layer_detection_response, FileMeta, GraphBuilder,
    LlmLayerResponse,
};
use ua_core::{Complexity, KnowledgeGraph, NodeType};

fn skeleton(paths: &[&str]) -> KnowledgeGraph {
    let mut b = GraphBuilder::new("demo", "");
    for p in paths {
        b.add_file(
            p,
            FileMeta {
                summary: String::new(),
                tags: Vec::new(),
                complexity: Complexity::Simple,
            },
        );
    }
    b.build("now")
}

#[test]
fn parser_strips_markdown_fences() {
    let raw = "```json\n[{\"name\":\"API\",\"description\":\"http\",\"filePatterns\":[\"src/api/\"]}]\n```";
    let parsed = parse_layer_detection_response(raw).expect("fenced json parses");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "API");
    assert_eq!(parsed[0].file_patterns, vec!["src/api/".to_string()]);
}

#[test]
fn parser_accepts_raw_json() {
    let raw =
        r#"[{"name":"UI","description":"front-end","filePatterns":["src/ui/","src/views/"]}]"#;
    let parsed = parse_layer_detection_response(raw).expect("raw json parses");
    assert_eq!(parsed[0].file_patterns.len(), 2);
}

#[test]
fn parser_salvages_array_inside_garbage_text() {
    // The model occasionally prefixes / suffixes a one-liner explanation —
    // `extract_first_array` should still find the JSON body.
    let raw = "Sure! Here is your JSON:\n[{\"name\":\"Data\",\"filePatterns\":[\"db/\"]}]\nLet me know if you need anything else.";
    let parsed = parse_layer_detection_response(raw).expect("salvage from prose");
    assert_eq!(parsed[0].name, "Data");
    // `description` is optional in the schema; defaults to empty.
    assert_eq!(parsed[0].description, "");
}

#[test]
fn parser_returns_none_for_garbage() {
    assert!(parse_layer_detection_response("").is_none());
    assert!(parse_layer_detection_response("not json at all").is_none());
    // Empty array survives parsing but `parse_layer_detection_response`
    // explicitly returns `None` when nothing usable came out.
    assert!(parse_layer_detection_response("[]").is_none());
}

#[test]
fn apply_llm_layers_assigns_each_file_exactly_once() {
    let graph = skeleton(&[
        "src/api/users.ts",
        "src/api/auth.ts",
        "src/services/payments.ts",
        "src/lib/format.ts", // should land in "Other" (no matching pattern)
    ]);

    let llm = vec![
        LlmLayerResponse {
            name: "API".into(),
            description: "http".into(),
            file_patterns: vec!["src/api/".into()],
        },
        LlmLayerResponse {
            name: "Service".into(),
            description: "biz".into(),
            file_patterns: vec!["src/services/".into()],
        },
    ];

    let layers = apply_llm_layers(&graph, &llm);

    // Tally: every file node appears in exactly one layer.
    let mut tally = std::collections::HashMap::<String, usize>::new();
    for layer in &layers {
        for nid in &layer.node_ids {
            *tally.entry(nid.clone()).or_default() += 1;
        }
    }
    let file_count = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::File)
        .count();
    assert_eq!(tally.len(), file_count, "every file node placed exactly once");
    for (nid, count) in &tally {
        assert_eq!(*count, 1, "node {nid} appeared {count} times across layers");
    }

    // Specific layer membership.
    let api = layers.iter().find(|l| l.name == "API").unwrap();
    let service = layers.iter().find(|l| l.name == "Service").unwrap();
    let other = layers.iter().find(|l| l.name == "Other").unwrap();

    assert_eq!(api.node_ids.len(), 2, "two files in API");
    assert!(api.node_ids.contains(&"file:src/api/users.ts".to_string()));
    assert!(api.node_ids.contains(&"file:src/api/auth.ts".to_string()));

    assert_eq!(service.node_ids, vec!["file:src/services/payments.ts".to_string()]);

    assert_eq!(other.node_ids, vec!["file:src/lib/format.ts".to_string()]);
    // Default description for the synthesized bucket.
    assert_eq!(other.description, "Uncategorized files");
}
