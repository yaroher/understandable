use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(function name: (variable) @fn.name) @fn.def
(signature name: (variable) @fn.name) @fn.def

(data_type name: (name) @cls.name) @cls.def
(type_synomym name: (name) @cls.name) @cls.def
(class name: (name) @cls.name) @cls.def

(import (module) @imp.source) @imp.def
"#;

const CALL_QUERY: &str = r#"
(apply function: (_) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "haskell",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["function"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_haskell::LANGUAGE.into()
}
