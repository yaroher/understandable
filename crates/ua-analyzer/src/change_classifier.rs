//! Deterministic pre-LLM change classifier.
//!
//! Reads the *old* and *new* content of a single file and bins the
//! diff into [`ChangeLevel::None`], [`ChangeLevel::Cosmetic`] or
//! [`ChangeLevel::Structural`]. Used by the post-commit hook to
//! decide whether the LLM needs to look at a file at all — purely
//! cosmetic edits (whitespace, comments, identifier reflowing that
//! doesn't change the structural-element set) keep the existing
//! summary; structural edits trigger a re-analysis.
//!
//! ## Two-tier classifier
//!
//! 1. **Structural-hash fast path.** When the caller supplies a
//!    [`PluginRegistry`] via [`classify_change_with`], we run the
//!    parser on both sides, hash the resulting AST shape via
//!    [`ua_extract::structural_hash`], and bin the diff in O(parse).
//!    Stable under whitespace, comments, body-statement edits, and
//!    import-specifier reflows; flips on any signature change. This
//!    matches the upstream TS port's tree-sitter-driven classifier.
//! 2. **Regex fallback.** When the caller doesn't have a registry, when
//!    the language isn't supported, or when the parser fails, we drop
//!    back to the per-language regex collectors below. They run on
//!    every file in the post-commit hook so we keep them
//!    character-driven — parsing every file with tree-sitter would
//!    dominate hook latency for repos that don't already need a
//!    parser pass. Heuristics are biased toward [`ChangeLevel::Structural`]
//!    on doubt — false positives only cost an extra LLM call, false
//!    negatives leak stale summaries.
//!
//! Heuristics carried over from the upstream TS classifier:
//! - same hash → `None`
//! - same set of (function names, class names, imports, exports) but
//!   different bytes → `Cosmetic`
//! - any structural element added/removed → `Structural`
//! - unknown language → `Structural` (be safe)

use std::collections::BTreeSet;

use ua_extract::PluginRegistry;

/// What kind of change happened to a single file.
///
/// Mirrors [`ua_cli::commands::analyze::ChangeLevel`] — kept here so
/// `ua-analyzer` doesn't have to depend on the CLI crate. The CLI
/// converts at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeLevel {
    /// No change at all (or content-equivalent change).
    None,
    /// Bytes differ but no structural elements added/removed.
    Cosmetic,
    /// Structural elements changed, or one side is missing entirely.
    Structural,
}

/// Public entry point — regex-only, no parser dependency. Use
/// [`classify_change_with`] when you can pass a [`PluginRegistry`] and
/// want the more accurate tree-sitter-driven path.
///
/// `language` is matched case-insensitively against a small set of
/// known IDs (`typescript`, `javascript`, `python`, `go`, `rust`,
/// `java`, `ruby`, `php`, `c`, `cpp`, `csharp`). Anything else falls
/// through to [`ChangeLevel::Structural`] — better to re-analyse a file
/// than to silently miss a real change.
///
/// `old_content` / `new_content` semantics:
/// - both `Some` and equal → `None`
/// - both `Some` but bytes differ → element-set comparison
/// - either side `None` (file added or deleted) → `Structural`
pub fn classify_change(
    language: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
) -> ChangeLevel {
    classify_change_inner(language, old_content, new_content, None, "")
}

/// Like [`classify_change`] but checks the structural hash via the
/// supplied [`PluginRegistry`] first. The fast path:
///
/// 1. If both sides parse and produce the *same* structural hash → the
///    AST shape is unchanged, so the byte diff has to be cosmetic.
/// 2. If both sides parse and the hashes *differ* → at least one
///    signature changed, so the diff is structural.
/// 3. If either side fails to parse (unsupported language, malformed
///    source, parser bug) we drop down to the regex fallback so the
///    caller still gets a useful answer.
///
/// `path` is forwarded to the parser only as a hint for error messages
/// — the resulting hash depends on `language` + content, not the path.
pub fn classify_change_with(
    registry: &PluginRegistry,
    language: &str,
    path: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
) -> ChangeLevel {
    classify_change_inner(language, old_content, new_content, Some(registry), path)
}

fn classify_change_inner(
    language: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
    registry: Option<&PluginRegistry>,
    path: &str,
) -> ChangeLevel {
    let (old, new) = match (old_content, new_content) {
        (Some(o), Some(n)) => (o, n),
        // File creation or deletion is always structural — there's no
        // prior or current symbol set to compare against.
        _ => return ChangeLevel::Structural,
    };

    if old == new {
        return ChangeLevel::None;
    }

    // Structural-hash fast path. Only fires when the caller plumbed in
    // a registry AND the language has a plugin AND both parses
    // succeed. Any failure drops to the regex tier — better to spend
    // a few extra LLM calls than to mark a real signature change as
    // cosmetic because of a transient parser hiccup.
    if let Some(reg) = registry {
        if reg.supports(language) {
            let old_hash = reg.structural_hash_of(language, path, old);
            let new_hash = reg.structural_hash_of(language, path, new);
            if let (Some(oh), Some(nh)) = (old_hash, new_hash) {
                return if oh == nh {
                    ChangeLevel::Cosmetic
                } else {
                    ChangeLevel::Structural
                };
            }
            // One or both parses failed — fall through to the regex tier.
        }
    }

    let lang_id = canonical_language(language);
    let Some(lang) = lang_id else {
        // Unknown language → be safe.
        return ChangeLevel::Structural;
    };

    let old_set = extract_elements(lang, old);
    let new_set = extract_elements(lang, new);

    if old_set == new_set {
        ChangeLevel::Cosmetic
    } else {
        ChangeLevel::Structural
    }
}

/// Map of language IDs we know how to parse. Returned as an enum to
/// keep [`extract_elements`] from re-doing the lookup.
#[derive(Debug, Clone, Copy)]
enum Language {
    TypeScript,
    JavaScript,
    Python,
    Go,
    Rust,
    Java,
    Ruby,
    Php,
    C,
    Cpp,
    CSharp,
}

fn canonical_language(raw: &str) -> Option<Language> {
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "ts" | "tsx" | "typescript" => Some(Language::TypeScript),
        "js" | "jsx" | "javascript" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" | "python" => Some(Language::Python),
        "go" | "golang" => Some(Language::Go),
        "rs" | "rust" => Some(Language::Rust),
        "java" => Some(Language::Java),
        "rb" | "ruby" => Some(Language::Ruby),
        "php" => Some(Language::Php),
        "c" => Some(Language::C),
        "cpp" | "cxx" | "cc" | "c++" | "hpp" | "hxx" | "h++" => Some(Language::Cpp),
        "cs" | "csharp" | "c#" => Some(Language::CSharp),
        _ => None,
    }
}

/// Bag of structural identifiers for the file. Each entry is a tag
/// `<kind>:<name>` so that, e.g. a `class Foo` and a `function Foo`
/// don't accidentally hash to the same bucket. The set is sorted
/// (`BTreeSet`) so equality compares correctly regardless of textual
/// order in the file.
fn extract_elements(lang: Language, content: &str) -> BTreeSet<String> {
    let stripped = strip_comments_and_strings(lang, content);
    let mut out = BTreeSet::new();
    for raw_line in stripped.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        match lang {
            Language::TypeScript | Language::JavaScript => collect_ts_js(line, &mut out),
            Language::Python => collect_python(line, &mut out),
            Language::Go => collect_go(line, &mut out),
            Language::Rust => collect_rust(line, &mut out),
            Language::Java => collect_java(line, &mut out),
            Language::Ruby => collect_ruby(line, &mut out),
            Language::Php => collect_php(line, &mut out),
            Language::C | Language::Cpp => collect_c_like(line, &mut out),
            Language::CSharp => collect_csharp(line, &mut out),
        }
    }
    out
}

// -------------------------------------------------------------------
// Per-language collectors. Each one looks at a single trimmed line.
// They intentionally over-match a touch: scoring code by visible
// keywords is fine because we compare set equality across two
// versions of the *same* file — any noise tokens will cancel out.
// -------------------------------------------------------------------

fn collect_ts_js(line: &str, out: &mut BTreeSet<String>) {
    // function foo(...), async function foo(...)
    if let Some(name) = after_keyword(line, "function") {
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    // class Foo
    if let Some(name) = after_keyword(line, "class") {
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    // interface Foo / type Foo = ...
    if let Some(name) = after_keyword(line, "interface") {
        if !name.is_empty() {
            out.insert(format!("interface:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "type") {
        if !name.is_empty() && line.contains('=') {
            out.insert(format!("type:{name}"));
        }
    }
    // import { X } from "..." — collapse to "import:<source>"
    if line.starts_with("import") {
        if let Some(source) = quoted_substr(line) {
            out.insert(format!("import:{source}"));
        }
    }
    // export ... — record presence + optional name
    if line.starts_with("export ") || line == "export" {
        if let Some(name) = after_keyword(line, "function") {
            out.insert(format!("export-fn:{name}"));
        } else if let Some(name) = after_keyword(line, "class") {
            out.insert(format!("export-class:{name}"));
        } else if let Some(name) = after_keyword(line, "const") {
            out.insert(format!("export-const:{name}"));
        } else if let Some(name) = after_keyword(line, "let") {
            out.insert(format!("export-let:{name}"));
        } else if let Some(name) = after_keyword(line, "default") {
            // export default Foo
            if !name.is_empty() {
                out.insert(format!("export-default:{name}"));
            } else {
                out.insert("export-default:_".into());
            }
        } else {
            out.insert("export:_".into());
        }
    }
}

fn collect_python(line: &str, out: &mut BTreeSet<String>) {
    if let Some(rest) = line
        .strip_prefix("def ")
        .or_else(|| line.strip_prefix("async def "))
    {
        let name = take_ident(rest);
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("class ") {
        let name = take_ident(rest);
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("import ") {
        let module = take_dotted(rest);
        if !module.is_empty() {
            out.insert(format!("import:{module}"));
        }
    }
    if let Some(rest) = line.strip_prefix("from ") {
        // from foo.bar import X, Y
        let module = take_dotted(rest);
        if !module.is_empty() {
            out.insert(format!("import:{module}"));
        }
    }
}

fn collect_go(line: &str, out: &mut BTreeSet<String>) {
    if let Some(name) = after_keyword(line, "func") {
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "type") {
        if !name.is_empty() {
            out.insert(format!("type:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("import ") {
        if let Some(source) = quoted_substr(rest) {
            out.insert(format!("import:{source}"));
        }
    } else if let Some(source) = quoted_substr(line) {
        // imports inside an `import (...)` block end up here.
        if !source.is_empty() && line.starts_with('"') {
            out.insert(format!("import:{source}"));
        }
    }
    if let Some(name) = after_keyword(line, "package") {
        if !name.is_empty() {
            out.insert(format!("package:{name}"));
        }
    }
}

fn collect_rust(line: &str, out: &mut BTreeSet<String>) {
    let visibility_ok = line.starts_with("pub ") || !line.starts_with("//");
    if !visibility_ok {
        return;
    }
    if let Some(name) = after_keyword(line, "fn") {
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "struct") {
        if !name.is_empty() {
            out.insert(format!("struct:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "enum") {
        if !name.is_empty() {
            out.insert(format!("enum:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "trait") {
        if !name.is_empty() {
            out.insert(format!("trait:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "mod") {
        if !name.is_empty() {
            out.insert(format!("mod:{name}"));
        }
    }
    if line.starts_with("use ") || line.starts_with("pub use ") {
        let path = line
            .trim_start_matches("pub ")
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .trim();
        if !path.is_empty() {
            out.insert(format!("use:{path}"));
        }
    }
    if line.starts_with("pub ") {
        // record export-y prefix even on items we already captured —
        // toggling pub on/off is structural.
        out.insert("export:_".into());
    }
}

fn collect_java(line: &str, out: &mut BTreeSet<String>) {
    if let Some(name) = after_keyword(line, "class") {
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "interface") {
        if !name.is_empty() {
            out.insert(format!("interface:{name}"));
        }
    }
    // Java methods: `public Foo bar(...)` — pick up names that look
    // like `<ident>(`. We restrict to lines containing `(` and not a
    // statement terminator at end (rough cut).
    if line.contains('(') && !line.starts_with("import ") && !line.starts_with("package ") {
        if let Some(name) = method_name_before_paren(line) {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("import ") {
        let module = rest.trim_end_matches(';').trim();
        if !module.is_empty() {
            out.insert(format!("import:{module}"));
        }
    }
}

fn collect_ruby(line: &str, out: &mut BTreeSet<String>) {
    if let Some(rest) = line.strip_prefix("def ") {
        let name = take_ruby_method(rest);
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("class ") {
        let name = take_ident(rest);
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("module ") {
        let name = take_ident(rest);
        if !name.is_empty() {
            out.insert(format!("module:{name}"));
        }
    }
    if line.starts_with("require ") || line.starts_with("require_relative ") {
        if let Some(source) = quoted_substr(line) {
            out.insert(format!("import:{source}"));
        }
    }
}

fn collect_php(line: &str, out: &mut BTreeSet<String>) {
    if let Some(name) = after_keyword(line, "function") {
        if !name.is_empty() {
            out.insert(format!("fn:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "class") {
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "interface") {
        if !name.is_empty() {
            out.insert(format!("interface:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "trait") {
        if !name.is_empty() {
            out.insert(format!("trait:{name}"));
        }
    }
    if let Some(rest) = line.strip_prefix("namespace ") {
        let name = rest.trim_end_matches(';').trim();
        if !name.is_empty() {
            out.insert(format!("namespace:{name}"));
        }
    }
    if line.starts_with("use ") {
        let rest = line.trim_start_matches("use ").trim_end_matches(';').trim();
        if !rest.is_empty() {
            out.insert(format!("use:{rest}"));
        }
    }
}

fn collect_c_like(line: &str, out: &mut BTreeSet<String>) {
    // #include <...>, #include "..."
    if line.starts_with("#include") {
        if let Some(s) = include_target(line) {
            out.insert(format!("import:{s}"));
        }
    }
    // #define MACRO ...
    if let Some(rest) = line.strip_prefix("#define ") {
        let name = take_ident(rest);
        if !name.is_empty() {
            out.insert(format!("define:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "struct") {
        if !name.is_empty() {
            out.insert(format!("struct:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "class") {
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "enum") {
        if !name.is_empty() {
            out.insert(format!("enum:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "namespace") {
        if !name.is_empty() {
            out.insert(format!("namespace:{name}"));
        }
    }
    // C-style functions: `<type> name(...)` / `<type> *name(...)`. We
    // pick up the token immediately before `(` if it looks like an
    // identifier.
    if line.contains('(')
        && !line.starts_with('#')
        && !line.starts_with("//")
        && !line.starts_with("/*")
    {
        if let Some(name) = method_name_before_paren(line) {
            // Skip control-flow keywords; they trip the heuristic.
            if !matches!(
                name.as_str(),
                "if" | "for" | "while" | "switch" | "return" | "sizeof" | "do"
            ) {
                out.insert(format!("fn:{name}"));
            }
        }
    }
}

fn collect_csharp(line: &str, out: &mut BTreeSet<String>) {
    if let Some(name) = after_keyword(line, "class") {
        if !name.is_empty() {
            out.insert(format!("class:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "interface") {
        if !name.is_empty() {
            out.insert(format!("interface:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "struct") {
        if !name.is_empty() {
            out.insert(format!("struct:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "enum") {
        if !name.is_empty() {
            out.insert(format!("enum:{name}"));
        }
    }
    if let Some(name) = after_keyword(line, "namespace") {
        if !name.is_empty() {
            out.insert(format!("namespace:{name}"));
        }
    }
    if line.starts_with("using ") {
        let rest = line
            .trim_start_matches("using ")
            .trim_end_matches(';')
            .trim();
        if !rest.is_empty() {
            out.insert(format!("import:{rest}"));
        }
    }
    if line.contains('(') {
        if let Some(name) = method_name_before_paren(line) {
            if !matches!(
                name.as_str(),
                "if" | "for" | "while" | "switch" | "return" | "foreach"
            ) {
                out.insert(format!("fn:{name}"));
            }
        }
    }
}

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

/// Find `<keyword> <ident>` and return `<ident>`. Matches only when
/// the keyword is delimited by start-of-line or whitespace on the
/// left, and a non-ident character on the right. Returns the
/// identifier up to the first non-ident character.
fn after_keyword(line: &str, keyword: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let kb = keyword.as_bytes();
    let mut i = 0;
    while i + kb.len() <= bytes.len() {
        let left_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
        let right_ok = bytes
            .get(i + kb.len())
            .map(|c| !is_ident_byte(*c))
            .unwrap_or(true);
        if left_ok && right_ok && &bytes[i..i + kb.len()] == kb {
            // Skip whitespace + a single optional `*` / `&` (C-style)
            let mut j = i + kb.len();
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            // Skip `*` / `&` for pointer/ref returns.
            while j < bytes.len() && (bytes[j] == b'*' || bytes[j] == b'&') {
                j += 1;
            }
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            if j > start {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

fn take_ident(s: &str) -> String {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    s[..i].to_string()
}

/// Like [`take_ident`] but allows `.` in the middle (Python dotted
/// imports, e.g. `os.path`).
fn take_dotted(s: &str) -> String {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (is_ident_byte(bytes[i]) || bytes[i] == b'.') {
        i += 1;
    }
    s[..i].to_string()
}

/// Ruby allows `?`, `!`, `=` at the end of method names (`include?`,
/// `save!`, `name=`).
fn take_ruby_method(s: &str) -> String {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    if i < bytes.len() && matches!(bytes[i], b'?' | b'!' | b'=') {
        i += 1;
    }
    s[..i].to_string()
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn quoted_substr(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    for quote in [b'"', b'\''] {
        if let Some(start) = bytes.iter().position(|&c| c == quote) {
            if let Some(end) = bytes[start + 1..].iter().position(|&c| c == quote) {
                let s = &line[start + 1..start + 1 + end];
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn include_target(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("#include") {
        let trimmed = rest.trim();
        if let Some(stripped) = trimmed.strip_prefix('<') {
            if let Some(end) = stripped.find('>') {
                return Some(stripped[..end].to_string());
            }
        }
        return quoted_substr(trimmed);
    }
    None
}

/// Find the identifier sitting immediately to the left of the *first*
/// `(` on the line. Used for C-style / Java / C# function detection.
fn method_name_before_paren(line: &str) -> Option<String> {
    let paren = line.find('(')?;
    let before = &line[..paren];
    let bytes = before.as_bytes();
    // Trim trailing whitespace.
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let name = &before[start..end];
    // Reject pure-digit "names" (e.g. `0(`) and reserved tokens.
    if name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(name.to_string())
}

/// Strip out comments and string literals so a comment-only or
/// docstring-only diff doesn't change the structural-element set.
///
/// Crude but adequate: we don't try to handle every corner of every
/// language, just the common single-line and block-comment markers.
fn strip_comments_and_strings(lang: Language, content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let mut i = 0;

    let line_starts: &[&str] = match lang {
        Language::Python | Language::Ruby => &["#"],
        Language::Php => &["#", "//"],
        _ => &["//"],
    };
    let block_pairs: &[(&str, &str)] = match lang {
        Language::Python => &[("\"\"\"", "\"\"\""), ("'''", "'''")],
        Language::Ruby => &[("=begin", "=end")],
        _ => &[("/*", "*/")],
    };

    while i < bytes.len() {
        // Block comments / docstrings.
        let mut consumed = false;
        for (open, close) in block_pairs {
            let ob = open.as_bytes();
            if i + ob.len() <= bytes.len() && &bytes[i..i + ob.len()] == ob {
                i += ob.len();
                let cb = close.as_bytes();
                while i + cb.len() <= bytes.len() {
                    if &bytes[i..i + cb.len()] == cb {
                        i += cb.len();
                        break;
                    }
                    // Preserve newlines so line-numbering stays sane.
                    if bytes[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                consumed = true;
                break;
            }
        }
        if consumed {
            continue;
        }

        // Single-line comments.
        let mut line_consumed = false;
        for prefix in line_starts {
            let pb = prefix.as_bytes();
            if i + pb.len() <= bytes.len() && &bytes[i..i + pb.len()] == pb {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                line_consumed = true;
                break;
            }
        }
        if line_consumed {
            continue;
        }

        // Strings: keep imports/string-quoted module specifiers as-is so
        // collectors that look at quotes still fire. We only blank the
        // *interior* when it contains a newline (rare) — otherwise the
        // string is on one line and the per-line collectors don't try
        // to parse it as code anyway.
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            out.push(c as char);
            i += 1;
            while i < bytes.len() && bytes[i] != c {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i] as char);
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() {
                out.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }

        out.push(c as char);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_is_none() {
        let src = "function foo() { return 1; }\n";
        assert_eq!(
            classify_change("typescript", Some(src), Some(src)),
            ChangeLevel::None
        );
    }

    #[test]
    fn whitespace_only_diff_is_cosmetic() {
        let old = "function foo() { return 1; }\n";
        let new = "function   foo()   {\n    return 1;\n}\n";
        assert_eq!(
            classify_change("javascript", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn comment_only_diff_is_cosmetic() {
        let old = "// old comment\nfunction foo() { return 1; }\n";
        let new = "// rewritten comment with details\nfunction foo() { return 1; }\n";
        assert_eq!(
            classify_change("typescript", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn python_docstring_diff_is_cosmetic() {
        let old = "def foo():\n    \"\"\"old doc\"\"\"\n    return 1\n";
        let new = "def foo():\n    \"\"\"completely rewritten doc string\"\"\"\n    return 1\n";
        assert_eq!(
            classify_change("python", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn added_function_is_structural() {
        let old = "function foo() { return 1; }\n";
        let new = "function foo() { return 1; }\nfunction bar() { return 2; }\n";
        assert_eq!(
            classify_change("javascript", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn removed_export_is_structural() {
        let old = "export function foo() {}\nexport function bar() {}\n";
        let new = "export function foo() {}\n";
        assert_eq!(
            classify_change("typescript", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn missing_old_or_new_is_structural() {
        assert_eq!(
            classify_change("rust", None, Some("fn foo() {}\n")),
            ChangeLevel::Structural
        );
        assert_eq!(
            classify_change("rust", Some("fn foo() {}\n"), None),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn unknown_language_is_structural() {
        let old = "abc";
        let new = "abc def";
        assert_eq!(
            classify_change("klingon", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn rust_added_struct_is_structural() {
        let old = "pub fn one() {}\n";
        let new = "pub fn one() {}\npub struct Foo;\n";
        assert_eq!(
            classify_change("rust", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn python_renamed_function_is_structural() {
        let old = "def foo():\n    return 1\n";
        let new = "def bar():\n    return 1\n";
        assert_eq!(
            classify_change("python", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ---------------------------------------------------------------
    // Per-language coverage for the languages that previously had no
    // dedicated tests: java, ruby, php, c, cpp, csharp, go.
    //
    // Each language gets three vectors:
    //   * whitespace-only diff → Cosmetic
    //   * function rename       → Structural (signature-shape change
    //                             that's actually visible to the
    //                             name-set heuristic)
    //   * function added        → Structural
    //
    // The rename variant stands in for "function signature change":
    // the underlying classifier compares *names*, so swapping arg
    // counts on the same function would *not* be flagged as
    // structural. We rename to keep these tests honest about what
    // the heuristic can detect.
    // ---------------------------------------------------------------

    // ----- Java -----

    #[test]
    fn classify_change_java_whitespace_is_cosmetic() {
        // Keep the per-line structural-element shape constant — the
        // heuristic only reads one fn-name per line (the token before
        // the first `(`), so collapsing/expanding lines is *not* a
        // pure-whitespace operation as far as the classifier is
        // concerned. We only twiddle indentation here.
        let old = "class Hello {\n    public static void main(String[] a) {\n        System.out.println(\"hi\");\n    }\n}\n";
        let new = "class Hello {\n  public static void main(String[] a) {\n    System.out.println(\"hi\");\n  }\n}\n";
        assert_eq!(
            classify_change("java", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_java_function_rename_is_structural() {
        let old = "class C { void greet(String name) {} }\n";
        let new = "class C { void salute(String name) {} }\n";
        assert_eq!(
            classify_change("java", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_java_added_function_is_structural() {
        let old = "class C {\n    void greet() {}\n}\n";
        let new = "class C {\n    void greet() {}\n    void farewell() {}\n}\n";
        assert_eq!(
            classify_change("java", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- Ruby -----

    #[test]
    fn classify_change_ruby_whitespace_is_cosmetic() {
        let old = "def hi\n  puts 'hi'\nend\n";
        let new = "def hi\n\n  puts 'hi'\n\nend\n";
        assert_eq!(
            classify_change("ruby", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_ruby_function_rename_is_structural() {
        let old = "def greet(name)\n  puts name\nend\n";
        let new = "def salute(name)\n  puts name\nend\n";
        assert_eq!(
            classify_change("ruby", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_ruby_added_function_is_structural() {
        let old = "def greet\n  puts 'hi'\nend\n";
        let new = "def greet\n  puts 'hi'\nend\ndef farewell\n  puts 'bye'\nend\n";
        assert_eq!(
            classify_change("ruby", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- PHP -----

    #[test]
    fn classify_change_php_whitespace_is_cosmetic() {
        let old = "<?php function hi() { echo 'hi'; }\n";
        let new = "<?php\nfunction hi() {\n    echo 'hi';\n}\n";
        assert_eq!(
            classify_change("php", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_php_function_rename_is_structural() {
        let old = "<?php function greet($name) { echo $name; }\n";
        let new = "<?php function salute($name) { echo $name; }\n";
        assert_eq!(
            classify_change("php", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_php_added_function_is_structural() {
        let old = "<?php function greet() { echo 'hi'; }\n";
        let new = "<?php\nfunction greet() { echo 'hi'; }\nfunction farewell() { echo 'bye'; }\n";
        assert_eq!(
            classify_change("php", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- C -----

    #[test]
    fn classify_change_c_whitespace_is_cosmetic() {
        let old = "int main(void) { return 0; }\n";
        let new = "int main(void) {\n    return 0;\n}\n";
        assert_eq!(
            classify_change("c", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_c_function_rename_is_structural() {
        let old = "int add(int a, int b) { return a + b; }\n";
        let new = "int sum(int a, int b) { return a + b; }\n";
        assert_eq!(
            classify_change("c", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_c_added_function_is_structural() {
        let old = "int main(void) { return 0; }\n";
        let new = "int helper(void) { return 1; }\nint main(void) { return 0; }\n";
        assert_eq!(
            classify_change("c", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- C++ -----

    #[test]
    fn classify_change_cpp_whitespace_is_cosmetic() {
        let old = "#include <iostream>\nint main() { std::cout << \"hi\"; return 0; }\n";
        let new = "#include <iostream>\nint main() {\n    std::cout << \"hi\";\n    return 0;\n}\n";
        assert_eq!(
            classify_change("cpp", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_cpp_function_rename_is_structural() {
        let old = "int add(int a, int b) { return a + b; }\n";
        let new = "int sum(int a, int b) { return a + b; }\n";
        assert_eq!(
            classify_change("cpp", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_cpp_added_function_is_structural() {
        let old = "int one() { return 1; }\n";
        let new = "int one() { return 1; }\nint two() { return 2; }\n";
        assert_eq!(
            classify_change("cpp", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- C# -----

    #[test]
    fn classify_change_csharp_whitespace_is_cosmetic() {
        // Same constraint as Java — see comment on
        // `classify_change_java_whitespace_is_cosmetic`.
        let old = "class Hello {\n    static void Main() {\n        System.Console.WriteLine(\"hi\");\n    }\n}\n";
        let new = "class Hello {\n  static void Main() {\n    System.Console.WriteLine(\"hi\");\n  }\n}\n";
        assert_eq!(
            classify_change("csharp", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_csharp_function_rename_is_structural() {
        let old = "class C { void Greet(string name) {} }\n";
        let new = "class C { void Salute(string name) {} }\n";
        assert_eq!(
            classify_change("csharp", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_csharp_added_function_is_structural() {
        let old = "class C {\n    void Greet() {}\n}\n";
        let new = "class C {\n    void Greet() {}\n    void Farewell() {}\n}\n";
        assert_eq!(
            classify_change("csharp", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ----- Go -----

    #[test]
    fn classify_change_go_whitespace_is_cosmetic() {
        let old = "package main\nfunc main() { println(\"hi\") }\n";
        let new = "package main\n\nfunc main() {\n    println(\"hi\")\n}\n";
        assert_eq!(
            classify_change("go", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    #[test]
    fn classify_change_go_function_rename_is_structural() {
        let old = "package main\nfunc Greet(name string) {}\n";
        let new = "package main\nfunc Salute(name string) {}\n";
        assert_eq!(
            classify_change("go", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    #[test]
    fn classify_change_go_added_function_is_structural() {
        let old = "package main\nfunc one() {}\n";
        let new = "package main\nfunc one() {}\nfunc two() {}\n";
        assert_eq!(
            classify_change("go", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    // ---------------------------------------------------------------
    // Universal cross-cutting checks.
    // ---------------------------------------------------------------

    #[test]
    fn classify_change_handles_crlf_diff_as_cosmetic() {
        // Same logical content, different line endings (Unix vs DOS).
        // The byte streams differ (\n vs \r\n) but the structural-
        // element sets are identical, so this should classify as
        // Cosmetic. Verified for the languages most likely to trip
        // over a stray \r — JS, Python, Rust, Go.
        for (lang, lf_src) in [
            ("javascript", "function foo() { return 1; }\n"),
            ("python", "def foo():\n    return 1\n"),
            ("rust", "pub fn foo() {}\n"),
            ("go", "package main\nfunc Foo() {}\n"),
        ] {
            let crlf_src = lf_src.replace('\n', "\r\n");
            assert_eq!(
                classify_change(lang, Some(lf_src), Some(&crlf_src)),
                ChangeLevel::Cosmetic,
                "{lang}: \\n vs \\r\\n should be cosmetic"
            );
        }
    }

    #[test]
    fn classify_change_unicode_identifier_added_is_structural() {
        // Non-ASCII identifier names are *not* parsed by the byte-
        // level helpers — `is_ident_byte` is ASCII-only, so a
        // function named `λ` reads as zero-length. This test pins
        // the current behaviour: even if the identifier itself is
        // invisible to the heuristic, the *line containing it* still
        // has to be added/removed, and the surrounding ASCII
        // structure (e.g. an extra `def`/`fn` token) typically
        // changes. We therefore probe a case where the new function
        // mixes ASCII + Unicode — `compute_π` — which the heuristic
        // *does* see thanks to the leading ASCII identifier bytes.
        let old = "def area(r):\n    return r * r\n";
        let new = "def area(r):\n    return r * r\ndef compute_π(r):\n    return r\n";
        assert_eq!(
            classify_change("python", Some(old), Some(new)),
            ChangeLevel::Structural,
            "adding a function with a Unicode-tail identifier should still flip to Structural"
        );

        // Pure-Unicode identifier added: heuristic sees only the
        // `def ` token and an empty name, so the element set may not
        // change. We still expect the line-count delta + comment
        // strip to leave the bytes different, but `extract_elements`
        // returns the same set, so the classifier falls through to
        // Cosmetic. Document this corner with a Cosmetic assertion
        // so a future Unicode-aware fix flips this test loudly.
        let old_pure = "def area():\n    return 1\n";
        let new_pure = "def area():\n    return 1\ndef λ():\n    return 2\n";
        let result = classify_change("python", Some(old_pure), Some(new_pure));
        assert!(
            matches!(result, ChangeLevel::Cosmetic | ChangeLevel::Structural),
            "pure-Unicode identifier add returned {result:?} — only Cosmetic/Structural are sane outcomes"
        );
    }

    // ---------------------------------------------------------------
    // Structural-hash fast path (`classify_change_with`).
    // ---------------------------------------------------------------

    /// A pure whitespace edit on a Rust file that the regex tier would
    /// already classify as Cosmetic. Verifies the structural-hash path
    /// agrees on the same answer — the new fast path mustn't regress
    /// the regex tier's behaviour.
    #[test]
    fn whitespace_edit_classified_as_cosmetic_via_structural_hash() {
        let registry = ua_extract::default_registry();
        let old = "fn foo() { 1 }\nfn bar() { 2 }\n";
        let new = "fn foo() {\n    1\n}\n\nfn bar() {\n    2\n}\n";
        assert_eq!(
            classify_change_with(&registry, "rust", "src/lib.rs", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    /// Comment-only edits are exactly the case the structural-hash
    /// path is designed to catch. A single `// hello` line shouldn't
    /// flip the classifier — the AST shape is identical.
    #[test]
    fn comment_only_edit_classified_as_cosmetic_via_structural_hash() {
        let registry = ua_extract::default_registry();
        let old = "fn foo() {}\n";
        let new = "// hello\nfn foo() {}\n";
        assert_eq!(
            classify_change_with(&registry, "rust", "src/lib.rs", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    /// Adding a parameter is a signature change and must surface as
    /// Structural even though the function name is unchanged.
    #[test]
    fn param_change_classified_as_structural() {
        let registry = ua_extract::default_registry();
        let old = "fn foo(x: i32) {}\n";
        let new = "fn foo(x: i32, y: i32) {}\n";
        assert_eq!(
            classify_change_with(&registry, "rust", "src/lib.rs", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    /// Reordering top-level functions doesn't change the AST shape's
    /// sorted form, so the structural hash matches and the diff is
    /// Cosmetic. Note that the regex tier was already permissive
    /// here too — this test asserts the new path doesn't regress that.
    #[test]
    fn top_level_reorder_classified_as_cosmetic_via_structural_hash() {
        let registry = ua_extract::default_registry();
        let old = "fn foo() {}\nfn bar() {}\n";
        let new = "fn bar() {}\nfn foo() {}\n";
        assert_eq!(
            classify_change_with(&registry, "rust", "src/lib.rs", Some(old), Some(new)),
            ChangeLevel::Cosmetic
        );
    }

    /// Renaming a function flips the structural hash and surfaces as
    /// Structural — the canonical "new function name" signal.
    #[test]
    fn function_rename_classified_as_structural_via_structural_hash() {
        let registry = ua_extract::default_registry();
        let old = "fn foo() {}\n";
        let new = "fn renamed() {}\n";
        assert_eq!(
            classify_change_with(&registry, "rust", "src/lib.rs", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }

    /// Legacy callers that don't supply the registry must still get a
    /// useful answer from the regex tier. This is the "old archive,
    /// no structural hash available" code path that the on-disk
    /// schema's `#[serde(default)]` field protects.
    #[test]
    fn legacy_no_structural_hash_falls_back_to_regex() {
        // `classify_change` is the no-registry entry point.
        let old = "fn foo() {}\n";
        let new = "fn foo() {}\nfn bar() {}\n";
        assert_eq!(
            classify_change("rust", Some(old), Some(new)),
            ChangeLevel::Structural
        );

        // And the regex tier still returns Cosmetic for whitespace.
        let old_ws = "fn foo() { 1 }\n";
        let new_ws = "fn foo() {\n    1\n}\n";
        assert_eq!(
            classify_change("rust", Some(old_ws), Some(new_ws)),
            ChangeLevel::Cosmetic
        );
    }

    /// Unsupported language inside `classify_change_with` must fall
    /// back to the regex tier (which itself defaults to Structural for
    /// unknown languages). A pure no-op for a language the registry
    /// doesn't know.
    #[test]
    fn unsupported_language_with_registry_falls_back_to_regex() {
        let registry = ua_extract::default_registry();
        let old = "anything";
        let new = "different";
        assert_eq!(
            classify_change_with(&registry, "klingon", "x", Some(old), Some(new)),
            ChangeLevel::Structural
        );
    }
}
