use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition (identifier) @fn.name) @fn.def
(short_function_definition (identifier) @fn.name) @fn.def

(struct_definition name: (identifier) @cls.name) @cls.def
(abstract_definition name: (identifier) @cls.name) @cls.def

(import_statement) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "julia",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function_definition", "short_function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_julia::LANGUAGE.into()
}
