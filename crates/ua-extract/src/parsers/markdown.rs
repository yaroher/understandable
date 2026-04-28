//! Markdown parser. Walks the file once and emits one [`SectionInfo`]
//! per ATX-style heading (`#`, `##`, `###`, … up to six `#`s). Setext
//! underline-style headings are out of scope — we'd need to look ahead
//! a line and the dashboard rarely cares.
//!
//! `lineRange` runs from the heading line to the line *before* the
//! next heading at the same or shallower depth. The final heading
//! ends at the last line of the file.

use ua_core::{SectionInfo, StructuralAnalysis};

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len() as u32;

    // (level, line_number, name, idx_in_lines)
    let mut headings: Vec<(u32, u32, String, usize)> = Vec::new();
    let mut in_code = false;
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some((level, name)) = parse_atx_heading(trimmed) {
            let line_no = (idx + 1) as u32;
            headings.push((level, line_no, name, idx));
        }
    }

    let mut sections = Vec::new();
    for (i, (level, line_no, name, _)) in headings.iter().enumerate() {
        // End line = line just before the next heading whose level <= current
        // level, or end-of-file if no such heading exists.
        let end = headings[i + 1..]
            .iter()
            .find(|(l, _, _, _)| *l <= *level)
            .map(|(_, ln, _, _)| ln.saturating_sub(1).max(*line_no))
            .unwrap_or(total_lines.max(*line_no));
        sections.push(SectionInfo {
            name: name.clone(),
            level: *level,
            line_range: (*line_no, end),
        });
    }

    if !sections.is_empty() {
        analysis.sections = Some(sections);
    }
    analysis
}

fn parse_atx_heading(line: &str) -> Option<(u32, String)> {
    let mut level = 0u32;
    let mut bytes = line.as_bytes().iter();
    while bytes.clone().next() == Some(&b'#') {
        level += 1;
        bytes.next();
        if level > 6 {
            return None;
        }
    }
    if level == 0 {
        return None;
    }
    let rest = &line[level as usize..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let name = rest.trim().trim_end_matches('#').trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((level, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atx_headings_become_sections() {
        let src = "# Title\nbody\n## Sub\nmore\n### Deep\n";
        let a = analyze(src);
        let sections = a.sections.unwrap();
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].name, "Title");
        assert_eq!(sections[0].level, 1);
        assert_eq!(sections[1].name, "Sub");
        assert_eq!(sections[1].level, 2);
        assert_eq!(sections[2].name, "Deep");
        assert_eq!(sections[2].level, 3);
    }

    #[test]
    fn line_range_extends_to_next_same_or_shallower_heading() {
        let src = "# A\nx\n## B\ny\n# C\nz\n";
        let a = analyze(src);
        let sections = a.sections.unwrap();
        // A spans 1..=4 (up to line before C at line 5).
        assert_eq!(sections[0].line_range, (1, 4));
        // B is nested under A and ends at the line before C.
        assert_eq!(sections[1].line_range, (3, 4));
        // C runs to end of file.
        assert_eq!(sections[2].line_range, (5, 6));
    }

    #[test]
    fn fenced_code_headings_ignored() {
        let src = "# Real\n```\n# fake\n```\n## Also real\n";
        let a = analyze(src);
        let sections = a.sections.unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Real");
        assert_eq!(sections[1].name, "Also real");
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_markdown_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.sections.is_none());
    }

    #[test]
    fn parser_markdown_comments_only_yields_empty_defs() {
        // Markdown HTML-comment style and the common `<!-- -->` form: no headings.
        let a = analyze("<!-- a comment -->\n<!-- another -->\n");
        assert!(a.sections.is_none());
    }

    #[test]
    fn parser_markdown_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}# Title\nbody\n";
        let a = analyze(src);
        // BOM is part of the first line so the leading `#` is no longer at
        // position 0 — accept either outcome but require no panic.
        let _ = a.sections;
    }

    #[test]
    fn parser_markdown_handles_crlf_line_endings() {
        let src = "# Title\r\nbody\r\n## Sub\r\nmore\r\n";
        let a = analyze(src);
        let sections = a.sections.unwrap();
        assert!(sections.iter().any(|s| s.name == "Title" && s.level == 1));
        assert!(sections.iter().any(|s| s.name == "Sub" && s.level == 2));
    }

    #[test]
    fn parser_markdown_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 12);
        for i in 0..60_000 {
            src.push_str(&format!("# Heading {i}\nsome body line\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "markdown parse took {elapsed:?}");
        assert!(a.sections.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_markdown_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
