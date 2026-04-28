use ua_core::{EdgeType, NodeType};

#[test]
fn all_node_types_present() {
    assert_eq!(NodeType::ALL.len(), 21);
}

#[test]
fn all_edge_types_present() {
    assert_eq!(EdgeType::ALL.len(), 35);
}

#[test]
fn node_type_serializes_snake_case() {
    let json = serde_json::to_string(&NodeType::Function).unwrap();
    assert_eq!(json, "\"function\"");
    let json = serde_json::to_string(&NodeType::Article).unwrap();
    assert_eq!(json, "\"article\"");
}

#[test]
fn edge_type_serializes_snake_case() {
    let json = serde_json::to_string(&EdgeType::DefinesSchema).unwrap();
    assert_eq!(json, "\"defines_schema\"");
    let json = serde_json::to_string(&EdgeType::ContainsFlow).unwrap();
    assert_eq!(json, "\"contains_flow\"");
}
