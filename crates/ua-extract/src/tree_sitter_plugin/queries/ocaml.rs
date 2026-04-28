use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(value_definition (let_binding pattern: (value_name) @fn.name)) @fn.def

(module_definition (module_binding name: (module_name) @cls.name)) @cls.def

(open_module (module_path) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(application_expression) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "ocaml",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["value_definition", "fun_expression"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_ocaml::LANGUAGE_OCAML.into()
}
