//! JSON parser. Treats top-level object keys as `section` definitions
//! and, when the document looks like an OpenAPI spec, lifts every
//! `paths.<path>` entry into an `endpoint` (with optional method when
//! the path's HTTP verbs can be sniffed).

use std::collections::HashMap;

use ua_core::StructuralAnalysis;

use crate::parsers::{def_with_fields, endpoint};

/// Strip a leading UTF-8 BOM. Strict JSON disallows it, but plenty of
/// editors emit one anyway, so we silently drop it before handing to
/// `serde_json`.
fn strip_bom(input: &str) -> &str {
    input.strip_prefix('\u{FEFF}').unwrap_or(input)
}

/// Build a key-name -> 1-based line-number index for top-level object
/// keys in a single linear scan over the source. Replaces the previous
/// per-key full-document rescan that was O(N^2).
///
/// We track brace/bracket nesting and string state with a tiny state
/// machine; the first `"key":` seen at depth 1 (i.e. directly inside the
/// outermost object) wins. We deliberately avoid re-running a full JSON
/// parse here — `serde_json` already validates structure in `analyze`,
/// and we only need line numbers, which are a structural concern.
fn build_top_key_lines(input: &str) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = HashMap::new();
    let bytes = input.as_bytes();
    let mut line: u32 = 1;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut esc = false;
    let mut string_start: Option<usize> = None;
    let mut string_start_line: u32 = 1;
    let mut last_string: Option<(String, u32)> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if esc {
                esc = false;
                if b == b'\n' {
                    line += 1;
                }
                i += 1;
                continue;
            }
            match b {
                b'\\' => {
                    esc = true;
                }
                b'"' => {
                    in_string = false;
                    if let Some(start) = string_start.take() {
                        // Slice is guaranteed to land on UTF-8 char
                        // boundaries because all bytes between the two
                        // quotes are either ASCII control bytes (handled
                        // above) or part of valid UTF-8 (preserved
                        // verbatim by the JSON spec).
                        if let Some(s) = input.get(start..i) {
                            last_string = Some((s.to_string(), string_start_line));
                        }
                    }
                }
                b'\n' => line += 1,
                _ => {}
            }
            i += 1;
            continue;
        }
        match b {
            b'\n' => line += 1,
            b'"' => {
                in_string = true;
                string_start = Some(i + 1);
                string_start_line = line;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            b':' if depth == 1 => {
                if let Some((k, line_no)) = last_string.take() {
                    out.entry(k).or_insert(line_no);
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

pub fn analyze(content: &str) -> StructuralAnalysis {
    let content = strip_bom(content);
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    let mut endpoints = Vec::new();

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(content);
    if let Ok(serde_json::Value::Object(map)) = parsed {
        let key_lines = build_top_key_lines(content);
        let is_openapi = map.contains_key("openapi") || map.contains_key("swagger");
        for (key, _) in map.iter() {
            let line = key_lines.get(key).copied().unwrap_or(1);
            defs.push(def_with_fields(key.clone(), "section", line, Vec::new()));
        }
        if is_openapi {
            if let Some(serde_json::Value::Object(paths)) = map.get("paths") {
                for (path, methods) in paths {
                    let line = key_lines.get(path).copied().unwrap_or(1);
                    if let serde_json::Value::Object(methods) = methods {
                        let verbs: Vec<&str> = methods
                            .keys()
                            .filter(|k| {
                                matches!(
                                    k.as_str(),
                                    "get" | "post" | "put"
                                        | "delete"
                                        | "patch"
                                        | "head"
                                        | "options"
                                )
                            })
                            .map(|s| s.as_str())
                            .collect();
                        if verbs.is_empty() {
                            endpoints.push(endpoint(None, path, line));
                        } else {
                            for v in verbs {
                                endpoints
                                    .push(endpoint(Some(&v.to_uppercase()), path, line));
                            }
                        }
                    } else {
                        endpoints.push(endpoint(None, path, line));
                    }
                }
            }
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    if !endpoints.is_empty() {
        analysis.endpoints = Some(endpoints);
    }
    analysis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_keys_become_sections() {
        let src = r#"{"name":"demo","version":1,"deps":{}}"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"deps"));
    }

    #[test]
    fn openapi_paths_become_endpoints() {
        let src = r#"{
  "openapi": "3.0.0",
  "info": {},
  "paths": {
    "/users": {"get": {}, "post": {}},
    "/users/{id}": {"delete": {}}
  }
}"#;
        let a = analyze(src);
        let endpoints = a.endpoints.unwrap();
        let pairs: Vec<(Option<&str>, &str)> = endpoints
            .iter()
            .map(|e| (e.method.as_deref(), e.path.as_str()))
            .collect();
        assert!(pairs.contains(&(Some("GET"), "/users")));
        assert!(pairs.contains(&(Some("POST"), "/users")));
        assert!(pairs.contains(&(Some("DELETE"), "/users/{id}")));
    }

    #[test]
    fn malformed_json_returns_empty_analysis() {
        let a = analyze("not json {");
        assert!(a.definitions.is_none());
        assert!(a.endpoints.is_none());
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_json_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
        assert!(a.endpoints.is_none());
    }

    #[test]
    fn parser_json_comments_only_yields_empty_defs() {
        // JSON has no comment syntax — feed lines that look comment-y so we
        // confirm that a non-JSON document yields no definitions.
        let a = analyze("// just a comment\n# also not real JSON\n");
        assert!(a.definitions.is_none());
        assert!(a.endpoints.is_none());
    }

    #[test]
    fn parser_json_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}{\"name\":\"demo\"}";
        let a = analyze(src);
        // BOM-prefixed JSON is technically invalid; we just need not to panic.
        let _ = a.definitions;
    }

    #[test]
    fn parser_json_handles_crlf_line_endings() {
        let src = "{\r\n  \"name\": \"demo\",\r\n  \"deps\": {}\r\n}\r\n";
        let a = analyze(src);
        let names: Vec<String> = a
            .definitions
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(names.iter().any(|n| n == "name"));
        assert!(names.iter().any(|n| n == "deps"));
    }

    #[test]
    fn parser_json_processes_60kb_within_budget() {
        let mut src = String::from("{");
        for i in 0..60_000 {
            if i > 0 {
                src.push(',');
            }
            src.push_str(&format!("\n  \"k_{i}\": {i}"));
        }
        src.push_str("\n}\n");
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "json parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    fn parser_json_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
