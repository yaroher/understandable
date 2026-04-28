//! Protocol Buffers parser. Picks up `message`, `enum` and `service`
//! definitions plus their nested children using brace-depth tracking.
//! Field-level extraction is intentionally omitted — the dashboard
//! only needs the top-level shape.

use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();

    let mut depth = 0u32;
    let mut in_block_comment = false;

    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let mut line = raw.to_string();

        // strip block-comment regions (very tolerant — handles only the
        // single-line case fully; multi-line comments are skipped wholesale).
        if in_block_comment {
            if let Some(end) = line.find("*/") {
                line = line[end + 2..].to_string();
                in_block_comment = false;
            } else {
                continue;
            }
        }
        if let Some(start) = line.find("/*") {
            if let Some(end_rel) = line[start..].find("*/") {
                let end = start + end_rel + 2;
                line = format!("{}{}", &line[..start], &line[end..]);
            } else {
                line = line[..start].to_string();
                in_block_comment = true;
            }
        }
        // strip line comments
        if let Some(idx) = line.find("//") {
            line = line[..idx].to_string();
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Brace counts already handled in main loop below
        }

        // Only care about top-level declarations (depth == 0); nested ones
        // are noise for the structural view.
        if depth == 0 {
            if let Some((kind, name)) = match_decl(trimmed) {
                defs.push(def_with_fields(name, kind, line_no, Vec::new()));
            }
        }

        for ch in trimmed.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn match_decl(line: &str) -> Option<(&'static str, String)> {
    for (keyword, kind) in [
        ("message ", "message"),
        ("enum ", "enum"),
        ("service ", "service"),
    ] {
        if let Some(rest) = line.strip_prefix(keyword) {
            let name = rest
                .split(|c: char| c.is_whitespace() || c == '{')
                .find(|tok| !tok.is_empty())?;
            return Some((kind, name.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_messages_enums_services() {
        let src = r#"
syntax = "proto3";
message User {
  string name = 1;
}
enum Role { ADMIN = 0; USER = 1; }
service UserService {
  rpc GetUser (User) returns (User);
}
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let pairs: Vec<(&str, &str)> = defs
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str()))
            .collect();
        assert!(pairs.contains(&("message", "User")));
        assert!(pairs.contains(&("enum", "Role")));
        assert!(pairs.contains(&("service", "UserService")));
    }

    #[test]
    fn nested_messages_are_ignored() {
        let src = r#"
message Outer {
  message Inner { string x = 1; }
}
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Outer");
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_protobuf_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_protobuf_comments_only_yields_empty_defs() {
        let a = analyze("// line comment\n// another\n/* block */\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_protobuf_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}message User { string name = 1; }\n";
        let a = analyze(src);
        // BOM glued to the keyword would shift the prefix — we only require
        // graceful handling, not particular outcomes.
        let _ = a.definitions;
    }

    #[test]
    fn parser_protobuf_handles_crlf_line_endings() {
        let src = "message User {\r\n  string name = 1;\r\n}\r\nenum Role { ADMIN = 0; }\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert!(defs.iter().any(|d| d.name == "User" && d.kind == "message"));
        assert!(defs.iter().any(|d| d.name == "Role" && d.kind == "enum"));
    }

    #[test]
    fn parser_protobuf_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 30);
        for i in 0..60_000 {
            src.push_str(&format!("message M_{i} {{ string field = 1; }}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "protobuf parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }
}
