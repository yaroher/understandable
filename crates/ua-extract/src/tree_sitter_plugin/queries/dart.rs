use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_signature name: (identifier) @fn.name) @fn.def
(method_signature) @fn.def

(class_definition name: (identifier) @cls.name) @cls.def

(import_or_export (library_import (uri (string_literal) @imp.source))) @imp.def
"#;

const CALL_QUERY: &str = r#"
(invocation_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "dart",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["class_body"],
    method_kinds: &["method_signature"],
    property_kinds: &["declaration"],
    function_node_kinds: &["function_signature", "method_signature"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_dart::LANGUAGE.into()
}
