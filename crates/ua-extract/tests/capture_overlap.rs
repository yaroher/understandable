//! Regression tests for the capture-mux bug in `analyze_file` and the
//! single-callee-per-match bug in `extract_call_graph`.
//!
//! See module-level docs in `tree_sitter_plugin.rs`.

use ua_extract::default_registry;

/// A file with both a top-level function declaration and a class
/// declaration (which contains a method) must surface BOTH a `fn`
/// declaration and a `cls` declaration. The previous `if/else if`
/// dispatch would silently swallow trailing captures inside a single
/// match — independent `if` checks fix that.
#[test]
fn ts_function_and_class_in_same_file_both_emit() {
    let src = r#"
function topLevel(a: number): number { return a; }

class Box {
    value: number = 0;
    set(n: number): void { this.value = n; }
}
"#;
    let r = default_registry();
    let a = r.analyze_file("typescript", "x.ts", src).unwrap();

    // The top-level function lands as a function decl.
    let fn_names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(fn_names.contains(&"topLevel"), "got {:?}", fn_names);
    // Class methods land as functions too (TS query captures
    // method_definition with @fn.* captures).
    assert!(fn_names.contains(&"set"), "got {:?}", fn_names);

    // The class also lands.
    let cls_names: Vec<&str> = a.classes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cls_names, vec!["Box"]);
    let box_cls = &a.classes[0];
    assert!(box_cls.methods.contains(&"set".to_string()));
    assert!(box_cls.properties.contains(&"value".to_string()));
}

/// `foo(bar())` is a nested call: two distinct call expressions, two
/// distinct callees (`foo` and `bar`). Confirm we emit one entry per
/// callee — i.e. the call graph isn't collapsed by a "last callee
/// wins" accumulator.
#[test]
fn ts_nested_call_emits_two_entries() {
    let src = r#"
function helper(): number { return 1; }
function other(): number { return 2; }
function entry(): number {
    return helper(other());
}
"#;
    let r = default_registry();
    let calls = r.extract_call_graph("typescript", "x.ts", src).unwrap();
    let from_entry: Vec<&str> = calls
        .iter()
        .filter(|c| c.caller == "entry")
        .map(|c| c.callee.as_str())
        .collect();
    assert!(from_entry.contains(&"helper"), "got {:?}", from_entry);
    assert!(from_entry.contains(&"other"), "got {:?}", from_entry);
    assert!(
        from_entry.len() >= 2,
        "expected at least 2 callees, got {:?}",
        from_entry
    );
}

/// A chained call like `foo()(...)` produces two distinct call
/// expressions in TS — once again at least two callees.
#[test]
fn ts_chained_call_emits_multiple_entries() {
    let src = r#"
function entry() {
    foo()(bar());
}
"#;
    let r = default_registry();
    let calls = r.extract_call_graph("typescript", "x.ts", src).unwrap();
    let callees: Vec<&str> = calls
        .iter()
        .filter(|c| c.caller == "entry")
        .map(|c| c.callee.as_str())
        .collect();
    // We expect at least 2 distinct call sites tracked.
    assert!(callees.len() >= 2, "got {:?}", callees);
    assert!(callees.contains(&"bar"), "got {:?}", callees);
}
