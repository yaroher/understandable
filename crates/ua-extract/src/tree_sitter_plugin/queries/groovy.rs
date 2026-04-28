use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(method_declaration name: (identifier) @fn.name) @fn.def

(class_declaration name: (identifier) @cls.name) @cls.def

(import_declaration) @imp.def
"#;

const CALL_QUERY: &str = r#"
(method_invocation name: (identifier) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "groovy",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["class_body"],
    method_kinds: &["method_declaration"],
    property_kinds: &["field_declaration"],
    function_node_kinds: &["method_declaration", "closure"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_groovy::LANGUAGE.into()
}
