use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition
  name: (name) @fn.name
  parameters: (formal_parameters) @fn.params) @fn.def

(method_declaration
  name: (name) @fn.name
  parameters: (formal_parameters) @fn.params) @fn.def

(class_declaration
  name: (name) @cls.name) @cls.def

(interface_declaration
  name: (name) @cls.name) @cls.def

(trait_declaration
  name: (name) @cls.name) @cls.def

(namespace_use_declaration
  (namespace_use_clause (qualified_name) @imp.source)) @imp.def
"#;

const CALL_QUERY: &str = r#"
(function_call_expression function: (_) @call.callee) @call.expr
(member_call_expression name: (name) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "php",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["declaration_list"],
    method_kinds: &["method_declaration"],
    property_kinds: &["property_declaration"],
    function_node_kinds: &[
        "function_definition",
        "method_declaration",
        "anonymous_function_creation_expression",
    ],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}
