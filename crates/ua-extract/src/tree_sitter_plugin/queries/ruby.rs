use crate::tree_sitter_plugin::LangSpec;

const STRUCTURAL: &str = r#"
(method
  name: (_) @fn.name) @fn.def

(singleton_method
  name: (_) @fn.name) @fn.def

(class
  name: (_) @cls.name) @cls.def

(module
  name: (_) @cls.name) @cls.def

(call method: (identifier) @_m
  (#match? @_m "^(require|require_relative|load)$")
  arguments: (argument_list (string) @imp.source)) @imp.def
"#;

const CALL_QUERY: &str = r#"
(call method: (identifier) @call.callee) @call.expr
(call method: (constant) @call.callee) @call.expr
"#;

pub const SPEC: LangSpec = LangSpec {
    id: "ruby",
    language: lang,
    structural_query: STRUCTURAL,
    call_query: CALL_QUERY,
    class_body_kinds: &["body_statement"],
    method_kinds: &["method", "singleton_method"],
    property_kinds: &["assignment", "instance_variable"],
    function_node_kinds: &["method", "singleton_method", "block", "lambda"],
};

fn lang() -> tree_sitter::Language {
    tree_sitter_ruby::LANGUAGE.into()
}
