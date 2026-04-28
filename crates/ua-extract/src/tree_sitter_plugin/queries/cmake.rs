use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_def (function_command (argument_list (argument) @fn.name))) @fn.def
(macro_def (macro_command (argument_list (argument) @fn.name))) @fn.def
"#;

const CALL_QUERY: &str = r#"
(normal_command (identifier) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "cmake",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_def", "macro_def"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_cmake::LANGUAGE.into()
}
