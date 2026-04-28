//! Round-trip the original tool's `knowledge-graph.json` shape through our
//! types: parse → re-serialize → re-parse must produce structurally identical
//! data, and the validator must accept it.

use std::path::PathBuf;

use ua_core::{validate_graph, KnowledgeGraph};

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/sample-graph.json");
    p
}

#[test]
fn parses_real_fixture() {
    let raw = std::fs::read_to_string(fixture_path()).expect("read fixture");
    let graph: KnowledgeGraph = serde_json::from_str(&raw).expect("parse fixture");

    assert_eq!(graph.nodes.len(), 97);
    assert_eq!(graph.edges.len(), 183);
    assert_eq!(graph.layers.len(), 7);
    assert_eq!(graph.tour.len(), 12);
    // The fixture is the demo graph that ships with the upstream tool
    // — keep its `project.name` literal so the roundtrip test exercises
    // real wire-format input rather than a renamed copy.
    assert_eq!(graph.project.name, "understand-anything");
}

#[test]
fn fixture_passes_validation() {
    let raw = std::fs::read_to_string(fixture_path()).unwrap();
    let graph: KnowledgeGraph = serde_json::from_str(&raw).unwrap();
    let report = validate_graph(&graph);
    assert!(
        report.is_valid(),
        "fixture failed validation: {:#?}",
        report.errors
    );
}

#[test]
fn roundtrip_preserves_structure() {
    let raw = std::fs::read_to_string(fixture_path()).unwrap();
    let graph_a: KnowledgeGraph = serde_json::from_str(&raw).unwrap();
    let serialized = serde_json::to_string(&graph_a).unwrap();
    let graph_b: KnowledgeGraph = serde_json::from_str(&serialized).unwrap();
    assert_eq!(graph_a, graph_b);
}
