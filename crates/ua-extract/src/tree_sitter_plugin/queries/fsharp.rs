use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_or_value_defn) @fn.def

(open_directive) @imp.def
"#;

const CALL_QUERY: &str = r#"
(application_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "fsharp",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_or_value_defn"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_fsharp::LANGUAGE_FSHARP.into()
}
