use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition name: (word) @fn.name) @fn.def

(command name: (command_name (word) @_n)
  (#match? @_n "^(source|\\.)$")
  argument: (_) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(command name: (command_name (word) @call.callee)) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "bash",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_bash::LANGUAGE.into()
}
