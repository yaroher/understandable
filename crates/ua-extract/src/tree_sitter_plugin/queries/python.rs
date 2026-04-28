use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition
  name: (identifier) @fn.name
  parameters: (parameters) @fn.params) @fn.def

(class_definition
  name: (identifier) @cls.name) @cls.def

(import_statement
  name: (dotted_name) @imp.source) @imp.def

(import_from_statement
  module_name: (dotted_name) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "python",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["block"],
    method_kinds: &["function_definition"],
    property_kinds: &["assignment"],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_python::LANGUAGE.into()
}
