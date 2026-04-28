use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(binding attrpath: (attrpath) @fn.name expression: (function_expression)) @fn.def
"#;

const CALL_QUERY: &str = r#"
(apply_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "nix",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_expression"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_nix::LANGUAGE.into()
}
