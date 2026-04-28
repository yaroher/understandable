use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(funcdef name: (identifier) @fn.name) @fn.def
"#;

const CALL_QUERY: &str = r#"
(funcname) @call.callee
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "jq",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["funcdef"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_jq::LANGUAGE.into()
}
