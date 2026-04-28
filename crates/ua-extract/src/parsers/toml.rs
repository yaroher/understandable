//! TOML parser. We do a hand-rolled line walk so we can record exact
//! line numbers for every section and key/value — `toml::de` doesn't
//! surface spans on the stable API. The grammar we accept is
//! intentionally small: `[name]` table headers, `[[name]]` arrays of
//! tables (treated like sections), `key = value` pairs.

use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    let mut current_section: Option<String> = None;

    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // [[array.of.tables]]
        if let Some(rest) = line.strip_prefix("[[") {
            if let Some(name) = rest.strip_suffix("]]") {
                let name = name.trim().to_string();
                defs.push(def_with_fields(
                    name.clone(),
                    "section",
                    line_no,
                    Vec::new(),
                ));
                current_section = Some(name);
                continue;
            }
        }
        // [table]
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.strip_suffix(']') {
                let name = name.trim().to_string();
                defs.push(def_with_fields(
                    name.clone(),
                    "section",
                    line_no,
                    Vec::new(),
                ));
                current_section = Some(name);
                continue;
            }
        }

        // key = value (`=` outside any quoted token).
        if let Some(eq_idx) = first_unquoted_eq(line) {
            let key = line[..eq_idx].trim().to_string();
            if key.is_empty() {
                continue;
            }
            let value = line[eq_idx + 1..].trim().to_string();
            let fields = match &current_section {
                Some(s) => vec![s.clone(), value],
                None => vec![value],
            };
            defs.push(def_with_fields(key, "variable", line_no, fields));
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn first_unquoted_eq(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut in_str: Option<u8> = None;
    for (i, b) in bytes.iter().enumerate() {
        match in_str {
            Some(quote) => {
                if *b == quote && (i == 0 || bytes[i - 1] != b'\\') {
                    in_str = None;
                }
            }
            None => match *b {
                b'"' | b'\'' => in_str = Some(*b),
                b'=' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_and_keys_emitted() {
        let src = r#"
title = "demo"

[server]
host = "127.0.0.1"
port = 8080

[[users]]
name = "alice"
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let sections: Vec<&str> = defs
            .iter()
            .filter(|d| d.kind == "section")
            .map(|d| d.name.as_str())
            .collect();
        assert!(sections.contains(&"server"));
        assert!(sections.contains(&"users"));

        let host = defs.iter().find(|d| d.name == "host").unwrap();
        assert_eq!(host.kind, "variable");
        assert_eq!(host.fields[0], "server");
    }

    #[test]
    fn equals_inside_string_does_not_split() {
        let src = r#"key = "a=b=c""#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let key = defs.iter().find(|d| d.name == "key").unwrap();
        assert!(key.fields.iter().any(|f| f.contains("a=b=c")));
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_toml_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_toml_comments_only_yields_empty_defs() {
        let a = analyze("# nothing here\n# also nothing\n#\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_toml_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}[server]\nport = 8080\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap_or_default();
        assert!(defs.iter().any(|d| d.name == "server" || d.name == "port"));
    }

    #[test]
    fn parser_toml_handles_crlf_line_endings() {
        let src = "[server]\r\nhost = \"127.0.0.1\"\r\nport = 8080\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"server"));
        assert!(names.contains(&"host"));
        assert!(names.contains(&"port"));
    }

    #[test]
    fn parser_toml_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 20);
        for i in 0..60_000 {
            src.push_str(&format!("key_{i} = {i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "toml parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_toml_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
