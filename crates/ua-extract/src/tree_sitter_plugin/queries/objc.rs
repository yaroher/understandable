use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(method_definition selector: (_) @fn.name) @fn.def

(class_interface name: (identifier) @cls.name) @cls.def
(class_implementation name: (identifier) @cls.name) @cls.def
(protocol_declaration name: (identifier) @cls.name) @cls.def

(import_directive) @imp.def
"#;

const CALL_QUERY: &str = r#"
(message_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "objc",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &["method_definition"],
    property_kinds: &["property_declaration"],
    function_node_kinds: &["method_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_objc::LANGUAGE.into()
}
