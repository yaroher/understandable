use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_declaration
  name: (identifier) @fn.name) @fn.def

(function_declaration
  name: (dot_index_expression) @fn.name) @fn.def
"#;

const CALL_QUERY: &str = r#"
(function_call name: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "lua",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_declaration", "function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_lua::LANGUAGE.into()
}
