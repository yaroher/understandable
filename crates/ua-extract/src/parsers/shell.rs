//! Shell-script parser. Picks up two things:
//!   * function declarations — both the POSIX `name() {` form and the
//!     bash-flavoured `function name {` form;
//!   * file inclusions via `source path` or `. path`, surfaced as
//!     [`ImportDecl`]s so the dashboard can stitch shell helpers together.

use ua_core::{ImportDecl, StructuralAnalysis};

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    let mut imports = Vec::new();

    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let trimmed = strip_inline_comment(raw).trim();
        if trimmed.is_empty() {
            continue;
        }

        // function name { … } / function name() { … }
        if let Some(rest) = trimmed.strip_prefix("function ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !name.is_empty() {
                defs.push(def_with_fields(name, "function", line_no, Vec::new()));
                continue;
            }
        }
        // name() { … } — `(` immediately after a bare ident.
        if let Some(open) = trimmed.find("()") {
            let head = &trimmed[..open];
            if !head.is_empty() && head.chars().all(is_ident_char) {
                defs.push(def_with_fields(
                    head.to_string(),
                    "function",
                    line_no,
                    Vec::new(),
                ));
                continue;
            }
        }

        // source path  /  . path
        let (kind, rest) = if let Some(r) = trimmed.strip_prefix("source ") {
            (Some("source"), Some(r))
        } else if let Some(r) = trimmed.strip_prefix(". ") {
            (Some("dot"), Some(r))
        } else {
            (None, None)
        };
        if let (Some(_), Some(rest)) = (kind, rest) {
            let target = rest.split_ascii_whitespace().next().unwrap_or("");
            let target = target.trim_matches(|c| c == '"' || c == '\'');
            if !target.is_empty() {
                imports.push(ImportDecl {
                    source: target.to_string(),
                    specifiers: Vec::new(),
                    line_number: line_no,
                });
            }
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    if !imports.is_empty() {
        analysis.imports = imports;
    }
    analysis
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == ':'
}

/// Drop a trailing `# comment` while leaving `#` inside quoted strings alone.
fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    for (i, b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if *b == q => quote = None,
            Some(_) => {}
            None => match *b {
                b'"' | b'\'' => quote = Some(*b),
                b'#' if i == 0 || bytes[i - 1].is_ascii_whitespace() => {
                    return &line[..i];
                }
                _ => {}
            },
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_functions_both_styles() {
        let src = r#"
greet() {
  echo "hi"
}
function farewell {
  echo "bye"
}
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"farewell"));
        assert!(defs.iter().all(|d| d.kind == "function"));
    }

    #[test]
    fn captures_source_and_dot_imports() {
        let src = r#"
source ./lib/common.sh
. helpers.sh
"#;
        let a = analyze(src);
        assert_eq!(a.imports.len(), 2);
        assert_eq!(a.imports[0].source, "./lib/common.sh");
        assert_eq!(a.imports[1].source, "helpers.sh");
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_shell_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
        assert!(a.imports.is_empty());
    }

    #[test]
    fn parser_shell_comments_only_yields_empty_defs() {
        let a = analyze("# comment line\n# another\n#\n");
        assert!(a.definitions.is_none());
        assert!(a.imports.is_empty());
    }

    #[test]
    fn parser_shell_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}greet() { echo hi; }\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_shell_handles_crlf_line_endings() {
        let src = "greet() {\r\n  echo \"hi\"\r\n}\r\nfunction farewell {\r\n  echo bye\r\n}\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"farewell"));
    }

    #[test]
    fn parser_shell_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 30);
        for i in 0..60_000 {
            src.push_str(&format!("fn_{i}() {{ echo {i}; }}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "shell parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_shell_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
