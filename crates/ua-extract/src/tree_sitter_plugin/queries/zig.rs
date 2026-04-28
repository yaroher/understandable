use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(FnProto (IDENTIFIER) @fn.name) @fn.def

(VarDecl (IDENTIFIER) @cls.name (StructDeclaration)) @cls.def
"#;

const CALL_QUERY: &str = r#"
(SuffixExpr (FnCallArguments)) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "zig",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["FnProto", "Decl"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_zig::LANGUAGE.into()
}
