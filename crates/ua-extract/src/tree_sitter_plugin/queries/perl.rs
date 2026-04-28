use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(subroutine_declaration_statement name: (bareword) @fn.name) @fn.def

(use_statement (package) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "perl",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["subroutine_declaration_statement"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_perl::LANGUAGE.into()
}
