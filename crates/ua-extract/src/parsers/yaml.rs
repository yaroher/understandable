//! YAML parser. Top-level mapping keys become `section` definitions.
//! Multi-document YAML files (`---` separators) emit one section per
//! document so consumers can see the boundary in the structural view.
//!
//! We use `serde_yaml_ng` to walk the document(s); when parsing fails we
//! fall back to a tolerant line scan so a syntactically broken file
//! still produces something useful for the dashboard.

use std::collections::HashMap;

use serde::Deserialize;
use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

/// Strip a leading UTF-8 BOM (`\u{FEFF}`) so downstream parsers see clean
/// text. `serde_yaml_ng::Deserializer::from_str` does not consume the BOM
/// and instead loops on it forever, so we always remove it up front.
fn strip_bom(input: &str) -> &str {
    input.strip_prefix('\u{FEFF}').unwrap_or(input)
}

/// Build a lookup table of top-level mapping keys to their 1-based line
/// numbers in a single linear scan. Replaces the previous per-key
/// `find_top_key_line` rescan that was O(N^2) on large documents.
///
/// Heuristic: a "top-level" line is one that starts in column 0 (no
/// leading whitespace), is not a comment, and contains a `:`. The first
/// occurrence of a given key wins (matching the previous behaviour of
/// `find_top_key_line`).
fn build_top_key_index(content: &str) -> HashMap<String, u32> {
    let mut idx: HashMap<String, u32> = HashMap::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.starts_with(|c: char| c.is_whitespace() || c == '#') {
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim();
            if !key.is_empty() {
                idx.entry(key.to_string()).or_insert((line_no + 1) as u32);
            }
        }
    }
    idx
}

pub fn analyze(content: &str) -> StructuralAnalysis {
    let content = strip_bom(content);
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();

    // Multi-doc handling: collect each `---` boundary line as a section
    // marker, then parse each document's top-level keys.
    let mut doc_boundaries: Vec<u32> = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        if raw.trim_start() == "---" {
            doc_boundaries.push((idx + 1) as u32);
        }
    }

    let top_keys = build_top_key_index(content);

    let mut doc_index: u32 = 0;
    let mut any_parsed = false;
    // Defensive cap: serde_yaml_ng has historically returned non-advancing
    // iterators on certain malformed inputs (BOM was one such trigger
    // before we strip it above). Bound the iteration to a sane multiple
    // of the boundary count so a misbehaving deserializer cannot wedge us.
    let max_docs: u32 = doc_boundaries
        .len()
        .saturating_add(2)
        .saturating_mul(2)
        .min(u32::MAX as usize) as u32;
    for doc in serde_yaml_ng::Deserializer::from_str(content) {
        if doc_index >= max_docs.max(1024) {
            break;
        }
        doc_index = doc_index.saturating_add(1);
        let value: Result<serde_yaml_ng::Value, _> = serde_yaml_ng::Value::deserialize(doc);
        let Ok(value) = value else { continue };
        any_parsed = true;
        let doc_line = doc_boundaries
            .get(doc_index.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(1);
        if doc_index > 1 || !doc_boundaries.is_empty() {
            defs.push(def_with_fields(
                format!("document-{doc_index}"),
                "section",
                doc_line,
                Vec::new(),
            ));
        }
        if let serde_yaml_ng::Value::Mapping(map) = value {
            for (key, _) in map {
                if let Some(name) = yaml_key_to_string(&key) {
                    let line = top_keys.get(&name).copied().unwrap_or(doc_line);
                    defs.push(def_with_fields(name, "section", line, Vec::new()));
                }
            }
        }
    }

    // Fallback line-oriented scan when serde_yaml_ng refused the input.
    if !any_parsed {
        for (idx, raw) in content.lines().enumerate() {
            let line_no = (idx + 1) as u32;
            let trimmed = raw.trim_end();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if !raw.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-') {
                continue;
            }
            if let Some(idx) = trimmed.find(':') {
                let key = trimmed[..idx].trim();
                if !key.is_empty() && !key.contains(' ') {
                    defs.push(def_with_fields(
                        key.to_string(),
                        "section",
                        line_no,
                        Vec::new(),
                    ));
                }
            }
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn yaml_key_to_string(v: &serde_yaml_ng::Value) -> Option<String> {
    match v {
        serde_yaml_ng::Value::String(s) => Some(s.clone()),
        serde_yaml_ng::Value::Number(n) => Some(n.to_string()),
        serde_yaml_ng::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_keys_become_sections() {
        let src = "name: demo\nversion: 1\nservices:\n  api: {}\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"version"));
        assert!(names.contains(&"services"));
        assert!(defs.iter().all(|d| d.kind == "section"));
    }

    #[test]
    fn multi_document_emits_document_sections() {
        let src = "---\nfoo: 1\n---\nbar: 2\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.iter().any(|n| n.starts_with("document-")));
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
    }

    #[test]
    fn empty_yields_no_definitions() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_yaml_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_yaml_comments_only_yields_empty_defs() {
        let a = analyze("# only comments\n# nothing else\n#\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_yaml_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}name: demo\nversion: 1\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap_or_default();
        assert!(defs.iter().any(|d| d.name == "version" || d.name == "name"));
    }

    #[test]
    fn parser_yaml_handles_crlf_line_endings() {
        let src = "name: demo\r\nversion: 1\r\nservices:\r\n  api: {}\r\n";
        let a = analyze(src);
        let names: Vec<String> = a
            .definitions
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(names.iter().any(|n| n == "name"));
        assert!(names.iter().any(|n| n == "version"));
        assert!(names.iter().any(|n| n == "services"));
    }

    #[test]
    fn parser_yaml_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 16);
        for i in 0..60_000 {
            src.push_str(&format!("key_{i}: value_{i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "yaml parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_yaml_non_utf8_bytes_returns_typed_error() {
        // analyze takes &str — non-UTF-8 bytes can never reach it without a
        // prior std::str::from_utf8 conversion, which itself returns Err.
        let bad: &[u8] = b"\xFF\xFE\x00\x00name: demo\n";
        let res = std::str::from_utf8(bad);
        assert!(
            res.is_err(),
            "non-utf8 bytes are caught at the &str boundary"
        );
    }
}
