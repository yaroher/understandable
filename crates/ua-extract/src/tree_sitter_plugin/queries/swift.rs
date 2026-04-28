use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_declaration name: (simple_identifier) @fn.name) @fn.def

(class_declaration name: (type_identifier) @cls.name) @cls.def
(protocol_declaration name: (type_identifier) @cls.name) @cls.def
(import_declaration (identifier) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "swift",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["class_body"],
    method_kinds: &["function_declaration"],
    property_kinds: &["property_declaration"],
    function_node_kinds: &["function_declaration"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_swift::LANGUAGE.into()
}
