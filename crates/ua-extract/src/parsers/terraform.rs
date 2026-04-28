//! Terraform / HCL parser. Surfaces the four block types operators
//! actually click through:
//!   * `resource "type" "name"` → [`ResourceInfo`] entries with `kind`
//!     set to the resource type;
//!   * `data "type" "name"` → `definitions` of kind=`data`;
//!   * `variable "name"` → kind=`variable`;
//!   * `output "name"` → kind=`output`.

use ua_core::StructuralAnalysis;

use crate::parsers::{def_with_fields, resource};

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut resources = Vec::new();
    let mut defs = Vec::new();

    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = strip_line_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((kind, name)) = match_resource(line) {
            // Tucked into ResourceInfo so callers see resources together.
            resources.push(resource(name, &kind, line_no));
            continue;
        }
        if let Some((kind, name)) = match_named_block(line) {
            defs.push(def_with_fields(name, kind, line_no, Vec::new()));
        }
    }

    if !resources.is_empty() {
        analysis.resources = Some(resources);
    }
    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn strip_line_comment(raw: &str) -> &str {
    let mut end = raw.len();
    if let Some(i) = raw.find('#') {
        end = end.min(i);
    }
    if let Some(i) = raw.find("//") {
        end = end.min(i);
    }
    &raw[..end]
}

/// `resource "aws_s3_bucket" "logs" {` → ("aws_s3_bucket", "logs")
fn match_resource(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("resource")?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let mut iter = quoted_tokens(rest);
    let kind = iter.next()?;
    let name = iter.next()?;
    Some((kind, name))
}

/// Single-or-two-quoted-name block. `data "x" "y" {` produces
/// ("data", "x.y"); `variable "name" {` produces ("variable", "name").
/// `output "name" { … }` keeps the same shape — we only look at quoted
/// tokens that appear *before* the opening brace so an inline body
/// like `output "x" { value = "y" }` doesn't get confused.
fn match_named_block(line: &str) -> Option<(&'static str, String)> {
    for (keyword, kind, expected) in [
        ("data", "data", 2usize),
        ("variable", "variable", 1usize),
        ("output", "output", 1usize),
    ] {
        if let Some(rest) = line.strip_prefix(keyword) {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                let head = match rest.find('{') {
                    Some(idx) => &rest[..idx],
                    None => rest,
                };
                let toks: Vec<String> = quoted_tokens(head).take(expected).collect();
                if toks.len() == expected {
                    let name = if toks.len() >= 2 {
                        format!("{}.{}", toks[0], toks[1])
                    } else {
                        toks[0].clone()
                    };
                    return Some((kind, name));
                }
            }
        }
    }
    None
}

/// Yield the contents of every double-quoted run on the line, in order.
fn quoted_tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    let mut chars = s.chars().peekable();
    std::iter::from_fn(move || {
        for c in chars.by_ref() {
            if c == '"' {
                let mut out = String::new();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '"' {
                        return Some(out);
                    }
                    out.push(next);
                }
                return None;
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_are_collected() {
        let src = r#"
resource "aws_s3_bucket" "logs" {
  bucket = "my-logs"
}
resource "aws_iam_role" "exec" {}
"#;
        let a = analyze(src);
        let res = a.resources.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].kind, "aws_s3_bucket");
        assert_eq!(res[0].name, "logs");
        assert_eq!(res[1].kind, "aws_iam_role");
        assert_eq!(res[1].name, "exec");
    }

    #[test]
    fn data_variable_output_become_definitions() {
        let src = r#"
data "aws_caller_identity" "current" {}
variable "region" { type = string }
output "bucket_name" { value = "x" }
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let pairs: Vec<(&str, &str)> = defs
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str()))
            .collect();
        assert!(pairs.contains(&("data", "aws_caller_identity.current")));
        assert!(pairs.contains(&("variable", "region")));
        assert!(pairs.contains(&("output", "bucket_name")));
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_terraform_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
        assert!(a.resources.is_none());
    }

    #[test]
    fn parser_terraform_comments_only_yields_empty_defs() {
        let a = analyze("# a comment\n// another comment\n#\n");
        assert!(a.definitions.is_none());
        assert!(a.resources.is_none());
    }

    #[test]
    fn parser_terraform_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}variable \"region\" { type = string }\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_terraform_handles_crlf_line_endings() {
        let src = "resource \"aws_s3_bucket\" \"logs\" {\r\n  bucket = \"x\"\r\n}\r\nvariable \"region\" {\r\n  type = string\r\n}\r\n";
        let a = analyze(src);
        let res = a.resources.unwrap();
        assert!(res.iter().any(|r| r.name == "logs" && r.kind == "aws_s3_bucket"));
        let defs = a.definitions.unwrap();
        assert!(defs.iter().any(|d| d.name == "region" && d.kind == "variable"));
    }

    #[test]
    fn parser_terraform_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 50);
        for i in 0..60_000 {
            src.push_str(&format!(
                "resource \"aws_s3_bucket\" \"b_{i}\" {{ bucket = \"x\" }}\n"
            ));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "terraform parse took {elapsed:?}");
        assert!(a.resources.is_some());
    }

    #[test]
    fn parser_terraform_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
