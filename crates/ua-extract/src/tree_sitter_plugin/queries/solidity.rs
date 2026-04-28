use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function_definition name: (identifier) @fn.name) @fn.def

(contract_declaration name: (identifier) @cls.name) @cls.def
(interface_declaration name: (identifier) @cls.name) @cls.def
(library_declaration name: (identifier) @cls.name) @cls.def

(import_directive) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call_expression function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "solidity",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["contract_body"],
    method_kinds: &["function_definition"],
    property_kinds: &["state_variable_declaration"],
    function_node_kinds: &["function_definition"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_solidity::LANGUAGE.into()
}
