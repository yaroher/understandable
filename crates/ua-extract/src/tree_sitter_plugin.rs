//! Tree-sitter analyzer plugin covering all tier-1 languages.
//!
//! Each language is described by a [`LangSpec`]: a tree-sitter [`Language`]
//! + a structural query (capture names: `fn.def`, `fn.name`, `fn.params`,
//!   `cls.def`, `cls.name`, `imp.def`, `imp.source`, `imp.spec`, `exp.def`,
//!   `exp.name`) + a call query (capture names: `call.expr`, `call.callee`).
//!
//! `analyze_file` runs the structural query, post-walks class bodies to
//! collect methods and properties, and resolves import string literals.
//! `extract_call_graph` runs the call query and pairs each call with the
//! enclosing function (the deepest function-like ancestor, identified via
//! the per-language `function_node_kinds` set).
//!
//! Bug-for-bug parity with the original imperative TS extractors is **not**
//! a goal — the goal is the same shape of output for the same source, with
//! enough fidelity that `ua-analyzer` builds an equivalent graph.
//!
//! ## Behaviour notes
//!
//! - **Capture mux is non-exclusive.** A single tree-sitter query match can
//!   carry several of the top-level capture groups (`fn.def`, `cls.def`,
//!   `imp.def`, `exp.def`) when patterns overlap (e.g. an `export class`).
//!   `analyze_file` iterates them with independent `if` checks rather than
//!   an `if/else if` chain, so a match emits *every* declaration kind it
//!   describes, not just the first one tested.
//! - **Multiple callees per match.** `extract_call_graph` collects every
//!   `@call.callee` capture in a match (a chained call like `foo()(bar())`
//!   yields two callees inside the outer `@call.expr`) and emits one
//!   `CallGraphEntry` per callee, pairing each callee with the *nearest*
//!   enclosing `@call.expr` ancestor for line attribution.

use std::collections::HashMap;
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator, Tree};
use ua_core::{
    CallGraphEntry, ClassDecl, Error, ExportDecl, FunctionDecl, ImportDecl, StructuralAnalysis,
};

use crate::plugin::{err_no_plugin, err_parse_failed, err_query, AnalyzerPlugin};

mod queries;

/// Static per-language description.
#[derive(Clone, Copy)]
pub(crate) struct LangSpec {
    pub id: &'static str,
    pub language: fn() -> Language,
    pub structural_query: &'static str,
    pub call_query: &'static str,
    pub class_body_kinds: &'static [&'static str],
    pub method_kinds: &'static [&'static str],
    pub property_kinds: &'static [&'static str],
    pub function_node_kinds: &'static [&'static str],
}

/// Compiled query + parser handle, cached per language.
struct CompiledLang {
    spec: LangSpec,
    language: Language,
    structural_query: Query,
    call_query: Query,
}

impl CompiledLang {
    fn new(spec: LangSpec) -> Result<Self, Error> {
        let language = (spec.language)();
        let structural_query = Query::new(&language, spec.structural_query)
            .map_err(|e| err_query(format!("{}: {e}", spec.id)))?;
        let call_query = Query::new(&language, spec.call_query)
            .map_err(|e| err_query(format!("{} call: {e}", spec.id)))?;
        Ok(Self {
            spec,
            language,
            structural_query,
            call_query,
        })
    }
}

#[non_exhaustive]
pub struct TreeSitterPlugin {
    by_lang: HashMap<&'static str, OnceLock<Result<CompiledLang, Error>>>,
    handled: Vec<&'static str>,
}

impl Default for TreeSitterPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSitterPlugin {
    pub fn new() -> Self {
        let mut by_lang = HashMap::new();
        let mut handled = Vec::new();
        for spec in all_specs() {
            by_lang.insert(spec.id, OnceLock::new());
            handled.push(spec.id);
        }
        Self { by_lang, handled }
    }

    fn compiled(&self, language: &str) -> Result<&CompiledLang, Error> {
        let cell = self
            .by_lang
            .get(language)
            .ok_or_else(|| err_no_plugin(language))?;
        let entry = cell.get_or_init(|| {
            let spec = all_specs()
                .iter()
                .find(|s| s.id == language)
                .copied()
                .ok_or_else(|| err_no_plugin(language))?;
            CompiledLang::new(spec)
        });
        match entry {
            Ok(c) => Ok(c),
            Err(e) => Err(err_query(format!("init {language}: {e}"))),
        }
    }
}

impl AnalyzerPlugin for TreeSitterPlugin {
    fn name(&self) -> &'static str {
        "tree-sitter"
    }

    fn languages(&self) -> &[&'static str] {
        &self.handled
    }

    fn analyze_file(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<StructuralAnalysis, Error> {
        let compiled = self.compiled(language)?;
        let tree = parse(&compiled.language, path, content)?;
        let mut analysis = StructuralAnalysis::default();
        let bytes = content.as_bytes();
        let root = tree.root_node();

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&compiled.structural_query, root, bytes);

        let names = compiled.structural_query.capture_names();

        // Track exported names so an `export class Foo {}` doesn't double-emit.
        let mut exported: std::collections::HashSet<String> = std::collections::HashSet::new();

        while let Some(m) = matches.next() {
            // Build a {capture-name -> node} dict for this match.
            let mut by_name: HashMap<&str, Node> = HashMap::new();
            for cap in m.captures.iter() {
                let cname = names[cap.index as usize];
                by_name.insert(cname, cap.node);
            }

            // Each top-level capture is checked *independently*. A single
            // match can carry several of {fn.def, cls.def, imp.def, exp.def}
            // simultaneously (e.g. when a query pattern is `(export_statement
            // declaration: (function_declaration ...) @fn.def) @exp.def`).
            // The previous `if/else if` chain silently dropped the trailing
            // captures.
            if let Some(def) = by_name.get("fn.def") {
                if let Some(name) = by_name.get("fn.name") {
                    let params = by_name
                        .get("fn.params")
                        .map(|p| collect_param_names(*p, bytes))
                        .unwrap_or_default();
                    analysis.functions.push(FunctionDecl {
                        name: text(*name, bytes).to_string(),
                        line_range: line_range(*def),
                        params,
                        return_type: None,
                    });
                }
            }
            if let Some(def) = by_name.get("cls.def") {
                if let Some(name) = by_name.get("cls.name") {
                    let (methods, properties) = collect_class_members(*def, compiled.spec, bytes);
                    analysis.classes.push(ClassDecl {
                        name: text(*name, bytes).to_string(),
                        line_range: line_range(*def),
                        methods,
                        properties,
                    });
                }
            }
            if let Some(def) = by_name.get("imp.def") {
                let source = by_name
                    .get("imp.source")
                    .map(|n| strip_quotes(text(*n, bytes)).to_string())
                    .unwrap_or_default();
                let mut specifiers = Vec::new();
                for cap in m.captures.iter() {
                    if names[cap.index as usize] == "imp.spec" {
                        specifiers.push(text(cap.node, bytes).to_string());
                    }
                }
                analysis.imports.push(ImportDecl {
                    source,
                    specifiers,
                    line_number: def.start_position().row as u32 + 1,
                });
            }
            if let Some(def) = by_name.get("exp.def") {
                if let Some(name) = by_name.get("exp.name") {
                    let n = text(*name, bytes).to_string();
                    if exported.insert(n.clone()) {
                        analysis.exports.push(ExportDecl {
                            name: n,
                            line_number: def.start_position().row as u32 + 1,
                        });
                    }
                }
            }
        }

        Ok(analysis)
    }

    fn extract_call_graph(
        &self,
        language: &str,
        path: &str,
        content: &str,
    ) -> Result<Vec<CallGraphEntry>, Error> {
        let compiled = match self.compiled(language) {
            Ok(c) => c,
            Err(_) => return Ok(Vec::new()),
        };
        let tree = parse(&compiled.language, path, content)?;
        let bytes = content.as_bytes();
        let root = tree.root_node();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&compiled.call_query, root, bytes);
        let names = compiled.call_query.capture_names();
        let mut out = Vec::new();
        while let Some(m) = matches.next() {
            // A single match can carry several `@call.callee` captures (for
            // instance the curried-call expression `foo()(bar())` parses as
            // one outer `call_expression` whose callee is itself a
            // `call_expression`, so two distinct callee positions are
            // captured at the same time). Likewise a match can carry
            // multiple `@call.expr` captures for nested call-expressions.
            // Emit one entry per callee; pair each callee with the closest
            // enclosing `@call.expr` ancestor so the line number tracks the
            // call site that physically contains it.
            let mut callees: Vec<Node> = Vec::new();
            let mut exprs: Vec<Node> = Vec::new();
            for cap in m.captures.iter() {
                match names[cap.index as usize] {
                    "call.callee" => callees.push(cap.node),
                    "call.expr" => exprs.push(cap.node),
                    _ => {}
                }
            }
            if callees.is_empty() || exprs.is_empty() {
                continue;
            }
            for callee in callees {
                let expr = nearest_call_expr_ancestor(callee, &exprs).unwrap_or(callee);
                let caller = enclosing_function_name(expr, compiled.spec, bytes)
                    .unwrap_or_else(|| "<top>".to_string());
                out.push(CallGraphEntry {
                    caller,
                    callee: text(callee, bytes).to_string(),
                    line_number: expr.start_position().row as u32 + 1,
                });
            }
        }
        Ok(out)
    }
}

fn parse(language: &Language, path: &str, content: &str) -> Result<Tree, Error> {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .map_err(|e| err_parse_failed(path, e.to_string()))?;
    parser
        .parse(content, None)
        .ok_or_else(|| err_parse_failed(path, "tree-sitter returned no tree"))
}

fn text<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    node.utf8_text(bytes).unwrap_or("")
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    let s = s.strip_prefix('\'').unwrap_or(s);
    let s = s.strip_suffix('\'').unwrap_or(s);
    s
}

fn line_range(node: Node<'_>) -> (u32, u32) {
    (
        node.start_position().row as u32 + 1,
        node.end_position().row as u32 + 1,
    )
}

fn collect_param_names(params: Node<'_>, bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        // Heuristic: pick first identifier descendent of each parameter.
        let kind = child.kind();
        if matches!(
            kind,
            "identifier"
                | "shorthand_property_identifier_pattern"
                | "property_identifier"
                | "type_identifier"
        ) {
            out.push(text(child, bytes).to_string());
        } else if let Some(name) = first_identifier_descendant(child) {
            out.push(text(name, bytes).to_string());
        } else {
            out.push(text(child, bytes).to_string());
        }
    }
    out
}

fn first_identifier_descendant<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    if matches!(
        node.kind(),
        "identifier"
            | "shorthand_property_identifier_pattern"
            | "property_identifier"
            | "type_identifier"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = first_identifier_descendant(child) {
            return Some(found);
        }
    }
    None
}

fn collect_class_members(
    class_node: Node<'_>,
    spec: LangSpec,
    bytes: &[u8],
) -> (Vec<String>, Vec<String>) {
    let mut methods = Vec::new();
    let mut properties = Vec::new();

    // Find the class body — first child whose kind is in `class_body_kinds`.
    let mut body: Option<Node<'_>> = None;
    let mut cursor = class_node.walk();
    for child in class_node.named_children(&mut cursor) {
        if spec.class_body_kinds.contains(&child.kind()) {
            body = Some(child);
            break;
        }
    }
    let Some(body) = body else {
        return (methods, properties);
    };

    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        let kind = member.kind();
        if spec.method_kinds.contains(&kind) {
            if let Some(name) = first_identifier_descendant(member) {
                methods.push(text(name, bytes).to_string());
            }
        } else if spec.property_kinds.contains(&kind) {
            if let Some(name) = first_identifier_descendant(member) {
                properties.push(text(name, bytes).to_string());
            }
        }
    }
    (methods, properties)
}

/// Find the `@call.expr` capture node that most-tightly encloses `callee`.
///
/// For a query whose pattern is `(call_expression function: (_) @call.callee)
/// @call.expr`, every `@call.callee` is by construction a child of *some*
/// `@call.expr` in the same match. When more than one `@call.expr` is
/// present (nested `call_expression` nodes share a match), pick the one
/// that's an ancestor of `callee` and that has the smallest byte span —
/// that's the call that physically wraps the callee identifier. Falls
/// back to the first `@call.expr` if the byte arithmetic finds no
/// ancestor (shouldn't happen for well-formed queries).
fn nearest_call_expr_ancestor<'tree>(
    callee: Node<'tree>,
    exprs: &[Node<'tree>],
) -> Option<Node<'tree>> {
    let cs = callee.start_byte();
    let ce = callee.end_byte();
    let mut best: Option<Node<'tree>> = None;
    for e in exprs {
        if e.start_byte() <= cs && e.end_byte() >= ce {
            best = Some(match best {
                Some(prev)
                    if prev.end_byte() - prev.start_byte() <= e.end_byte() - e.start_byte() =>
                {
                    prev
                }
                _ => *e,
            });
        }
    }
    best.or_else(|| exprs.first().copied())
}

fn enclosing_function_name(node: Node<'_>, spec: LangSpec, bytes: &[u8]) -> Option<String> {
    let mut cur = node.parent();
    while let Some(parent) = cur {
        if spec.function_node_kinds.contains(&parent.kind()) {
            // Try to find the name among children.
            let mut cursor = parent.walk();
            for child in parent.named_children(&mut cursor) {
                let k = child.kind();
                if matches!(k, "identifier" | "property_identifier" | "field_identifier") {
                    return Some(text(child, bytes).to_string());
                }
            }
            // Fallback: walk one level deeper for an identifier.
            if let Some(id) = first_identifier_descendant(parent) {
                return Some(text(id, bytes).to_string());
            }
            return Some(parent.kind().to_string());
        }
        cur = parent.parent();
    }
    None
}

// ---- Per-language spec table ----

fn all_specs() -> &'static [LangSpec] {
    static SPECS: OnceLock<Vec<LangSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        let mut v: Vec<LangSpec> = Vec::new();
        #[cfg(feature = "lang-typescript")]
        {
            v.push(queries::typescript::SPEC_TS);
            v.push(queries::typescript::SPEC_TSX);
        }
        #[cfg(feature = "lang-javascript")]
        {
            v.push(queries::javascript::SPEC_JS);
        }
        #[cfg(feature = "lang-python")]
        {
            v.push(queries::python::SPEC);
        }
        #[cfg(feature = "lang-go")]
        {
            v.push(queries::go::SPEC);
        }
        #[cfg(feature = "lang-rust")]
        {
            v.push(queries::rust_lang::SPEC);
        }
        #[cfg(feature = "lang-java")]
        {
            v.push(queries::java::SPEC);
        }
        #[cfg(feature = "lang-ruby")]
        {
            v.push(queries::ruby::SPEC);
        }
        #[cfg(feature = "lang-php")]
        {
            v.push(queries::php::SPEC);
        }
        #[cfg(feature = "lang-c")]
        {
            v.push(queries::c::SPEC);
        }
        #[cfg(feature = "lang-cpp")]
        {
            v.push(queries::cpp::SPEC);
        }
        #[cfg(feature = "lang-csharp")]
        {
            v.push(queries::csharp::SPEC);
        }

        // tier 2
        #[cfg(feature = "lang-bash")]
        v.push(queries::bash::SPEC);
        #[cfg(feature = "lang-lua")]
        v.push(queries::lua::SPEC);
        #[cfg(feature = "lang-zig")]
        v.push(queries::zig::SPEC);
        #[cfg(feature = "lang-dart")]
        v.push(queries::dart::SPEC);
        #[cfg(feature = "lang-swift")]
        v.push(queries::swift::SPEC);
        #[cfg(feature = "lang-scala")]
        v.push(queries::scala::SPEC);
        #[cfg(feature = "lang-haskell")]
        v.push(queries::haskell::SPEC);
        #[cfg(feature = "lang-ocaml")]
        v.push(queries::ocaml::SPEC);
        #[cfg(feature = "lang-elixir")]
        v.push(queries::elixir::SPEC);
        #[cfg(feature = "lang-erlang")]
        v.push(queries::erlang::SPEC);
        #[cfg(feature = "lang-elm")]
        v.push(queries::elm::SPEC);
        #[cfg(feature = "lang-julia")]
        v.push(queries::julia::SPEC);
        #[cfg(feature = "lang-scheme")]
        v.push(queries::scheme::SPEC);
        #[cfg(feature = "lang-solidity")]
        v.push(queries::solidity::SPEC);
        #[cfg(feature = "lang-perl")]
        v.push(queries::perl::SPEC);
        #[cfg(feature = "lang-fortran")]
        v.push(queries::fortran::SPEC);
        #[cfg(feature = "lang-d")]
        v.push(queries::d::SPEC);
        #[cfg(feature = "lang-fsharp")]
        v.push(queries::fsharp::SPEC);
        #[cfg(feature = "lang-groovy")]
        v.push(queries::groovy::SPEC);
        #[cfg(feature = "lang-objc")]
        v.push(queries::objc::SPEC);
        #[cfg(feature = "lang-cuda")]
        v.push(queries::cuda::SPEC);
        #[cfg(feature = "lang-glsl")]
        v.push(queries::glsl::SPEC);
        #[cfg(feature = "lang-hlsl")]
        v.push(queries::hlsl::SPEC);
        #[cfg(feature = "lang-verilog")]
        v.push(queries::verilog::SPEC);
        #[cfg(feature = "lang-vhdl")]
        v.push(queries::vhdl::SPEC);
        #[cfg(feature = "lang-cmake")]
        v.push(queries::cmake::SPEC);
        #[cfg(feature = "lang-make")]
        v.push(queries::make::SPEC);
        #[cfg(feature = "lang-nix")]
        v.push(queries::nix::SPEC);
        #[cfg(feature = "lang-vim")]
        v.push(queries::vim::SPEC);
        #[cfg(feature = "lang-fish")]
        v.push(queries::fish::SPEC);
        #[cfg(feature = "lang-jq")]
        v.push(queries::jq::SPEC);
        #[cfg(feature = "lang-hcl")]
        v.push(queries::hcl::SPEC);
        v
    })
}
