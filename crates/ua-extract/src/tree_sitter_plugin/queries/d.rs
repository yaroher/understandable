use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_declaration (identifier) @fn.name) @fn.def

(class_declaration (identifier) @cls.name) @cls.def
(struct_declaration (identifier) @cls.name) @cls.def
(interface_declaration (identifier) @cls.name) @cls.def

(import_declaration) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "d",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &["function_declaration"],
    property_kinds: &["variable_declaration"],
    function_node_kinds: &["function_declaration"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_d::LANGUAGE.into()
}
