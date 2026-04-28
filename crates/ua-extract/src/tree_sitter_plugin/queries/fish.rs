use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition name: (word) @fn.name) @fn.def
"#;

const CALL_QUERY: &str = r#"
(command name: (word) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "fish",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_fish::language()
}
