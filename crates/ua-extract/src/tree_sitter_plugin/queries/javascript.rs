use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_declaration
  name: (identifier) @fn.name
  parameters: (formal_parameters) @fn.params) @fn.def

(method_definition
  name: (property_identifier) @fn.name
  parameters: (formal_parameters) @fn.params) @fn.def

(variable_declarator
  name: (identifier) @fn.name
  value: [(arrow_function) (function_expression)] @_v) @fn.def

(class_declaration
  name: (identifier) @cls.name) @cls.def

(import_statement
  source: (string) @imp.source) @imp.def

(export_statement
  declaration: (function_declaration name: (identifier) @exp.name)) @exp.def

(export_statement
  declaration: (class_declaration name: (identifier) @exp.name)) @exp.def

(export_statement
  (export_clause (export_specifier name: (identifier) @exp.name))) @exp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC_JS: LangSpec = LangSpec {
    id: "javascript",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["class_body"],
    method_kinds: &["method_definition"],
    property_kinds: &["field_definition", "property_definition"],
    function_node_kinds: &[
        "function_declaration",
        "method_definition",
        "arrow_function",
        "function_expression",
        "function",
    ],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}
