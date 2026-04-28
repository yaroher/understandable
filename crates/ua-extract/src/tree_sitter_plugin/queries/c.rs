use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @fn.name
    parameters: (parameter_list) @fn.params)) @fn.def

(struct_specifier
  name: (type_identifier) @cls.name) @cls.def

(union_specifier
  name: (type_identifier) @cls.name) @cls.def

(enum_specifier
  name: (type_identifier) @cls.name) @cls.def

(preproc_include path: (_) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "c",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["field_declaration_list"],
    method_kinds: &[],
    property_kinds: &["field_declaration"],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_c::LANGUAGE.into()
}
