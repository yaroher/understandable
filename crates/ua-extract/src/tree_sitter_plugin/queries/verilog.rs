use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(module_declaration (module_header (simple_identifier) @cls.name)) @cls.def

(function_declaration (function_identifier) @fn.name) @fn.def
"#;

const CALL_QUERY: &str = r#"
(function_call) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "verilog",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_declaration"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_verilog::LANGUAGE.into()
}
