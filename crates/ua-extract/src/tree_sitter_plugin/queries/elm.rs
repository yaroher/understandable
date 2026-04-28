use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(value_declaration (function_declaration_left (lower_case_identifier) @fn.name)) @fn.def

(type_declaration (upper_case_identifier) @cls.name) @cls.def
(type_alias_declaration (upper_case_identifier) @cls.name) @cls.def

(import_clause (upper_case_qid) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(function_call_expr) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "elm",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["value_declaration"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_elm::LANGUAGE.into()
}
