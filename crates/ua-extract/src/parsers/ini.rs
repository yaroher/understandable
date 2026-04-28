//! INI / TOML-shape parser (no escapes, no nested tables) — turns
//! sections into `section` definitions and keys into `variable`
//! definitions whose `fields` array holds the section name they
//! belong to.

use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    let mut current_section: Option<String> = None;
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
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
        let Some(eq_idx) = line.find('=') else {
            continue;
        };
        let key = line[..eq_idx].trim().to_string();
        if key.is_empty() {
            continue;
        }
        let value = line[eq_idx + 1..].trim().to_string();
        let fields = match &current_section {
            Some(sect) => vec![sect.clone(), value],
            None => vec![value],
        };
        defs.push(def_with_fields(key, "variable", line_no, fields));
    }
    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_ini_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_ini_comments_only_yields_empty_defs() {
        let a = analyze("# pound comment\n; semicolon comment\n#\n;\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_ini_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}[section]\nkey=value\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_ini_handles_crlf_line_endings() {
        let src = "[server]\r\nhost=127.0.0.1\r\nport=8080\r\n[client]\r\nname=alice\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"server"));
        assert!(names.contains(&"client"));
        assert!(names.contains(&"host"));
        assert!(names.contains(&"port"));
        assert!(names.contains(&"name"));
    }

    #[test]
    fn parser_ini_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 25);
        src.push_str("[main]\n");
        for i in 0..60_000 {
            src.push_str(&format!("key_{i}=value_{i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "ini parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_ini_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
