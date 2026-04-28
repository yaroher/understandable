use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(subroutine name: (name) @fn.name) @fn.def
(function name: (name) @fn.name) @fn.def

(module_statement name: (name) @cls.name) @cls.def
"#;

const CALL_QUERY: &str = r#"
(call_expression name: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "fortran",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["subroutine", "function"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_fortran::LANGUAGE.into()
}
