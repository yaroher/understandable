use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(call target: (identifier) @_def
  (#match? @_def "^(def|defp|defmacro)$")
  (arguments (call target: (identifier) @fn.name))) @fn.def

(call target: (identifier) @_mod
  (#eq? @_mod "defmodule")
  (arguments (alias) @cls.name)) @cls.def

(call target: (identifier) @_imp
  (#match? @_imp "^(import|alias|use|require)$")
  (arguments (alias) @imp.source)) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call target: (identifier) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "elixir",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &[],
    method_kinds: &[],
    property_kinds: &[],
    function_node_kinds: &["call"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_elixir::LANGUAGE.into()
}
