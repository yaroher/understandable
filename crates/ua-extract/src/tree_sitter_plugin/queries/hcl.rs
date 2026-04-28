use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(block (identifier) @cls.name) @cls.def
"#;

const CALL_QUERY: &str = r#"
(function_call function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "hcl",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &[],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_hcl::LANGUAGE.into()
}
