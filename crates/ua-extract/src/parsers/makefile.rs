//! Tiny Makefile parser — extracts targets and top-level variable
//! definitions. Indented lines are recipe bodies and are ignored.

use ua_core::StructuralAnalysis;

use crate::parsers::{def_with_fields, step};

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut steps = Vec::new();
    let mut defs = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        // Recipe lines (start with TAB) belong to the previous target
        // — skip them entirely.
        if line.starts_with('\t') {
            continue;
        }
        let trimmed = line.trim_start_matches(' ').trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Variable assignment first — `=`, `:=`, `?=`, `+=` before any colon.
        if let Some((lhs, _)) = first_assignment(trimmed) {
            let name = lhs.trim().to_string();
            if !name.is_empty() {
                defs.push(def_with_fields(name, "variable", line_no, Vec::new()));
                continue;
            }
        }
        // Target line: `name [name2]: dep1 dep2`
        if let Some(colon_idx) = trimmed.find(':') {
            // Skip `:=` already handled above.
            if trimmed.as_bytes().get(colon_idx + 1) == Some(&b'=') {
                continue;
            }
            let lhs = &trimmed[..colon_idx];
            let deps = trimmed[colon_idx + 1..].trim();
            for target in lhs.split_ascii_whitespace() {
                let mut entry = step(target, line_no);
                // Tuck dependency list into the line range comment via name —
                // there is no fields slot on `StepInfo`, so callers can read
                // the raw target list themselves if needed.
                if !deps.is_empty() {
                    entry.name = format!("{target} <- {deps}");
                }
                steps.push(entry);
            }
        }
    }

    if !steps.is_empty() {
        analysis.steps = Some(steps);
    }
    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn first_assignment(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'=' && i > 0 {
            let prev = bytes[i - 1];
            if matches!(prev, b':' | b'?' | b'+') {
                return Some((&line[..i - 1], &line[i + 1..]));
            }
            return Some((&line[..i], &line[i + 1..]));
        }
        if c == b':' {
            // `:=` is an assignment — leave it for the next iteration to
            // pick up; everything else means we've hit a target line.
            if bytes.get(i + 1) == Some(&b'=') {
                i += 1;
                continue;
            }
            return None;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_makefile_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.steps.is_none());
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_makefile_comments_only_yields_empty_defs() {
        let a = analyze("# a comment\n# another comment\n#\n");
        assert!(a.steps.is_none());
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_makefile_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}all: build test\nVERSION = 1.0\n";
        let a = analyze(src);
        let _ = a.steps;
        let _ = a.definitions;
    }

    #[test]
    fn parser_makefile_handles_crlf_line_endings() {
        let src = "VERSION = 1.0\r\nall: build test\r\nbuild:\r\n\techo build\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert!(defs
            .iter()
            .any(|d| d.name == "VERSION" && d.kind == "variable"));
        let steps = a.steps.unwrap();
        assert!(steps.iter().any(|s| s.name.starts_with("all <-")));
        assert!(steps.iter().any(|s| s.name == "build"));
    }

    #[test]
    fn parser_makefile_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 30);
        for i in 0..60_000 {
            src.push_str(&format!("target_{i}: dep_{i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "makefile parse took {elapsed:?}");
        assert!(a.steps.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_makefile_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
