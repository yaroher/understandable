use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(rule (targets (word) @fn.name)) @fn.def
"#;

const CALL_QUERY: &str = r#"
(function_call name: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "makefile",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["rule"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_make::LANGUAGE.into()
}
