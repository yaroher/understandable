use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(fun_decl name: (atom) @fn.name) @fn.def
"#;

const CALL_QUERY: &str = r#"
(call expr: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "erlang",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["fun_decl"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_erlang::LANGUAGE.into()
}
