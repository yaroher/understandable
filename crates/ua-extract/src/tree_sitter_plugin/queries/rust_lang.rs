use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_item
  name: (identifier) @fn.name
  parameters: (parameters) @fn.params) @fn.def

(struct_item
  name: (type_identifier) @cls.name) @cls.def

(enum_item
  name: (type_identifier) @cls.name) @cls.def

(trait_item
  name: (type_identifier) @cls.name) @cls.def

(impl_item
  trait: (type_identifier) @cls.name) @cls.def

(use_declaration
  argument: (_) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr

(macro_invocation macro: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "rust",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["declaration_list", "field_declaration_list"],
    method_kinds: &["function_item"],
    property_kinds: &["field_declaration"],
    function_node_kinds: &["function_item", "closure_expression"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_rust::LANGUAGE.into()
}
