use serde_json::{json, Map, Value};
use ua_analyzer::{normalize_batch_output, normalize_complexity, normalize_node_id, DropReason};

fn ctx<'a>(
    node_type: &'a str,
    file_path: Option<&'a str>,
    name: Option<&'a str>,
) -> ua_analyzer::normalize::NormalizeContext<'a> {
    ua_analyzer::normalize::NormalizeContext {
        node_type,
        file_path,
        name,
        parent_flow_slug: None,
    }
}

#[test]
fn id_passes_through_when_already_canonical() {
    let id = "function:src/auth.ts:login";
    let out = normalize_node_id(id, ctx("function", Some("src/auth.ts"), Some("login")));
    assert_eq!(out, id);
}

#[test]
fn legacy_func_prefix_canonicalises_to_function() {
    let out = normalize_node_id(
        "func:src/auth.ts:login",
        ctx("function", Some("src/auth.ts"), Some("login")),
    );
    assert_eq!(out, "function:src/auth.ts:login");
}

#[test]
fn double_prefixed_id_is_collapsed() {
    let out = normalize_node_id(
        "function:function:src/auth.ts:login",
        ctx("function", Some("src/auth.ts"), Some("login")),
    );
    assert_eq!(out, "function:src/auth.ts:login");
}

#[test]
fn project_prefixed_id_is_stripped() {
    let out = normalize_node_id(
        "myproject:function:src/auth.ts:login",
        ctx("function", Some("src/auth.ts"), Some("login")),
    );
    assert_eq!(out, "function:src/auth.ts:login");
}

#[test]
fn bare_path_is_reconstructed() {
    let out = normalize_node_id(
        "src/auth.ts:login",
        ctx("function", Some("src/auth.ts"), Some("login")),
    );
    assert_eq!(out, "function:src/auth.ts:login");
}

#[test]
fn complexity_aliases_normalise() {
    assert_eq!(
        normalize_complexity(&Value::String("trivial".into())),
        "simple"
    );
    assert_eq!(
        normalize_complexity(&Value::String("Hard".into())),
        "complex"
    );
    assert_eq!(
        normalize_complexity(&Value::String("medium".into())),
        "moderate"
    );
    assert_eq!(normalize_complexity(&json!(2)), "simple");
    assert_eq!(normalize_complexity(&json!(5)), "moderate");
    assert_eq!(normalize_complexity(&json!(9)), "complex");
    assert_eq!(
        normalize_complexity(&Value::String("???".into())),
        "moderate"
    );
}

#[test]
fn complexity_simple_aliases() {
    for s in ["simple", "low", "easy", "trivial", "basic"] {
        assert_eq!(
            normalize_complexity(&Value::String(s.into())),
            "simple",
            "alias {s:?}"
        );
    }
}

#[test]
fn complexity_moderate_aliases() {
    for s in ["moderate", "medium", "intermediate", "mid", "average"] {
        assert_eq!(
            normalize_complexity(&Value::String(s.into())),
            "moderate",
            "alias {s:?}"
        );
    }
}

#[test]
fn complexity_complex_aliases() {
    for s in ["complex", "high", "hard", "difficult", "advanced"] {
        assert_eq!(
            normalize_complexity(&Value::String(s.into())),
            "complex",
            "alias {s:?}"
        );
    }
}

#[test]
fn complexity_aliases_are_case_insensitive_and_trimmed() {
    assert_eq!(
        normalize_complexity(&Value::String("  TRIVIAL  ".into())),
        "simple"
    );
    assert_eq!(
        normalize_complexity(&Value::String("\tIntermediate\n".into())),
        "moderate"
    );
    assert_eq!(
        normalize_complexity(&Value::String("ADVANCED".into())),
        "complex"
    );
}

#[test]
fn complexity_numeric_buckets_full_range() {
    // 1..=3 → simple
    assert_eq!(normalize_complexity(&json!(1)), "simple");
    assert_eq!(normalize_complexity(&json!(2)), "simple");
    assert_eq!(normalize_complexity(&json!(3)), "simple");
    // 4..=6 → moderate
    assert_eq!(normalize_complexity(&json!(4)), "moderate");
    assert_eq!(normalize_complexity(&json!(5)), "moderate");
    assert_eq!(normalize_complexity(&json!(6)), "moderate");
    // 7..=10 → complex
    assert_eq!(normalize_complexity(&json!(7)), "complex");
    assert_eq!(normalize_complexity(&json!(8)), "complex");
    assert_eq!(normalize_complexity(&json!(9)), "complex");
    assert_eq!(normalize_complexity(&json!(10)), "complex");
    // floats in the bucket boundaries.
    assert_eq!(normalize_complexity(&json!(2.5)), "simple");
    assert_eq!(normalize_complexity(&json!(3.0001)), "moderate");
    assert_eq!(normalize_complexity(&json!(6.0001)), "complex");
}

#[test]
fn complexity_unknown_string_falls_back_to_moderate() {
    assert_eq!(
        normalize_complexity(&Value::String("no idea".into())),
        "moderate"
    );
    assert_eq!(normalize_complexity(&Value::String("".into())), "moderate");
    assert_eq!(
        normalize_complexity(&Value::String("nightmare".into())),
        "moderate"
    );
}

#[test]
fn complexity_out_of_range_or_invalid_numeric_falls_back_to_moderate() {
    // Sub-1 numerics aren't in the bucket scheme — fall back.
    assert_eq!(normalize_complexity(&json!(0)), "moderate");
    assert_eq!(normalize_complexity(&json!(0.5)), "moderate");
    // Negative numerics likewise.
    assert_eq!(normalize_complexity(&json!(-1)), "moderate");
    // Bools / null / arrays — non-string, non-number — moderate.
    assert_eq!(normalize_complexity(&Value::Null), "moderate");
    assert_eq!(normalize_complexity(&json!(true)), "moderate");
    assert_eq!(normalize_complexity(&json!([])), "moderate");
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}

#[test]
fn batch_drops_dangling_edge_with_reason() {
    let nodes = vec![obj(json!({
        "id": "file:a.ts",
        "type": "file",
        "name": "a.ts",
        "filePath": "a.ts",
        "summary": "",
        "tags": [],
        "complexity": "simple"
    }))];
    let edges = vec![obj(json!({
        "source": "file:a.ts",
        "target": "file:does-not-exist.ts",
        "type": "imports",
        "direction": "forward",
        "weight": 0.7
    }))];

    let result = normalize_batch_output(nodes, edges);
    assert_eq!(result.edges.len(), 0);
    assert_eq!(result.stats.dangling_edges_dropped, 1);
    assert_eq!(
        result.stats.dropped_edges[0].reason,
        DropReason::MissingTarget
    );
}

#[test]
fn batch_rewrites_edges_after_id_fix() {
    let nodes = vec![
        obj(json!({
            "id": "func:a.ts:login",        // legacy short prefix
            "type": "function",
            "name": "login",
            "filePath": "a.ts",
            "summary": "",
            "tags": [],
            "complexity": "simple"
        })),
        obj(json!({
            "id": "function:a.ts:helper",
            "type": "function",
            "name": "helper",
            "filePath": "a.ts",
            "summary": "",
            "tags": [],
            "complexity": "simple"
        })),
    ];
    let edges = vec![obj(json!({
        "source": "func:a.ts:login",
        "target": "function:a.ts:helper",
        "type": "calls",
        "direction": "forward",
        "weight": 0.8
    }))];

    let result = normalize_batch_output(nodes, edges);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(
        result.edges[0].get("source").unwrap().as_str().unwrap(),
        "function:a.ts:login"
    );
    assert_eq!(result.stats.ids_fixed, 1);
    assert_eq!(result.stats.edges_rewritten, 1);
}

#[test]
fn batch_dedups_identical_edges() {
    let nodes = vec![
        obj(
            json!({"id":"file:a.ts","type":"file","name":"a","summary":"","tags":[],"complexity":"simple"}),
        ),
        obj(
            json!({"id":"file:b.ts","type":"file","name":"b","summary":"","tags":[],"complexity":"simple"}),
        ),
    ];
    let edges = vec![
        obj(
            json!({"source":"file:a.ts","target":"file:b.ts","type":"imports","direction":"forward","weight":0.7}),
        ),
        obj(
            json!({"source":"file:a.ts","target":"file:b.ts","type":"imports","direction":"forward","weight":0.7}),
        ),
    ];
    let result = normalize_batch_output(nodes, edges);
    assert_eq!(result.edges.len(), 1);
}
