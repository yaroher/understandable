use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition (function_declaration (identifier) @fn.name)) @fn.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "vim",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_vim::language()
}
