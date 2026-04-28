use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(method_declaration
  name: (identifier) @fn.name
  parameters: (parameter_list) @fn.params) @fn.def

(class_declaration
  name: (identifier) @cls.name) @cls.def

(interface_declaration
  name: (identifier) @cls.name) @cls.def

(struct_declaration
  name: (identifier) @cls.name) @cls.def

(using_directive (_) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(invocation_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "csharp",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["declaration_list"],
    method_kinds: &["method_declaration"],
    property_kinds: &["property_declaration", "field_declaration"],
    function_node_kinds: &["method_declaration", "lambda_expression"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}
