use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(list (symbol) @_kw
  (#match? @_kw "^(define|define-syntax)$")
  . (list . (symbol) @fn.name)) @fn.def
"#;

const CALL_QUERY: &str = r#"
(list . (symbol) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "scheme",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["list"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_scheme::LANGUAGE.into()
}
