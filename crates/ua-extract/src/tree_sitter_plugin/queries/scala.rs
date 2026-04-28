use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition name: (identifier) @fn.name) @fn.def
(function_declaration name: (identifier) @fn.name) @fn.def

(class_definition name: (identifier) @cls.name) @cls.def
(object_definition name: (identifier) @cls.name) @cls.def
(trait_definition name: (identifier) @cls.name) @cls.def

(import_declaration) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "scala",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["template_body"],
    method_kinds: &["function_definition"],
    property_kinds: &["val_definition", "var_definition"],
    function_node_kinds: &["function_definition", "function_declaration"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_scala::LANGUAGE.into()
}
