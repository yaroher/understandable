use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_declaration
  name: (identifier) @fn.name
  parameters: (parameter_list) @fn.params) @fn.def

(method_declaration
  name: (field_identifier) @fn.name
  parameters: (parameter_list) @fn.params) @fn.def

(type_declaration
  (type_spec name: (type_identifier) @cls.name)) @cls.def

(import_spec path: (interpreted_string_literal) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "go",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["struct_type", "field_declaration_list"],
    method_kinds: &["method_declaration"],
    property_kinds: &["field_declaration"],
    function_node_kinds: &["function_declaration", "method_declaration", "func_literal"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_go::LANGUAGE.into()
}
