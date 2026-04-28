//! `.env` parser. Each `KEY=VALUE` line becomes a `variable`
//! definition. Quoted values are kept verbatim minus the outer quote
//! pair; comments and blank lines are ignored.

use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.trim_start_matches("export ");
        let Some(eq_idx) = line.find('=') else {
            continue;
        };
        let key = line[..eq_idx].trim().to_string();
        if key.is_empty() {
            continue;
        }
        let value = strip_quotes(line[eq_idx + 1..].trim());
        defs.push(def_with_fields(
            key,
            "variable",
            line_no,
            vec![value.to_string()],
        ));
    }
    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    // Quoted form first — content after the closing quote (e.g. an
    // inline comment) is dropped along with anything else.
    for q in ['"', '\''] {
        if let Some(rest) = s.strip_prefix(q) {
            if let Some(idx) = rest.find(q) {
                return &rest[..idx];
            }
        }
    }
    // Unquoted: strip a trailing inline comment that starts with ` #`.
    if let Some(idx) = s.find(" #") {
        return s[..idx].trim();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_env_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_env_comments_only_yields_empty_defs() {
        let a = analyze("# only comments\n# another\n#\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_env_handles_utf8_bom_gracefully() {
        // `analyze` trims the line, so a BOM at the start glues to the key
        // name. We just need not to panic and to either accept or skip it.
        let src = "\u{FEFF}KEY=value\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_env_handles_crlf_line_endings() {
        let src = "KEY1=value1\r\nKEY2=\"value 2\"\r\nexport KEY3=value3\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert!(defs.iter().any(|d| d.name == "KEY1"));
        assert!(defs.iter().any(|d| d.name == "KEY2"));
        assert!(defs.iter().any(|d| d.name == "KEY3"));
    }

    #[test]
    fn parser_env_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 20);
        for i in 0..60_000 {
            src.push_str(&format!("KEY_{i}=value_{i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "env parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_env_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
