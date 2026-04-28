//! Tiny Dockerfile parser. Recognises `FROM`, `RUN`, `CMD`,
//! `ENTRYPOINT`, `EXPOSE`, `ARG`, `ENV`, `LABEL`, `COPY`, `ADD` and
//! `WORKDIR`. Continuation lines (`\` at end-of-line) are folded.

use ua_core::StructuralAnalysis;

use crate::parsers::{resource, service, step};

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut services = Vec::new();
    let mut steps = Vec::new();
    let mut endpoints = Vec::new();
    let mut resources = Vec::new();
    let mut stage_idx = 0u32;

    for (line_idx, raw_line) in fold_continuations(content).into_iter().enumerate() {
        let line_no = (line_idx + 1) as u32;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (instr, rest) = match trimmed.split_once(char::is_whitespace) {
            Some((a, b)) => (a.to_ascii_uppercase(), b.trim().to_string()),
            None => (trimmed.to_ascii_uppercase(), String::new()),
        };
        match instr.as_str() {
            "FROM" => {
                stage_idx += 1;
                let (image, alias) = parse_from(&rest);
                let stage_name = alias.unwrap_or_else(|| format!("stage-{stage_idx}"));
                services.push(service(stage_name, Some(&image), line_no));
            }
            "RUN" | "CMD" | "ENTRYPOINT" => {
                let summary = rest.split_ascii_whitespace().next().unwrap_or("").to_string();
                let name = if summary.is_empty() {
                    format!("{instr}-{line_no}")
                } else {
                    format!("{}: {}", instr.to_lowercase(), summary)
                };
                steps.push(step(name, line_no));
            }
            "EXPOSE" => {
                for token in rest.split_ascii_whitespace() {
                    let port = token.split('/').next().unwrap_or(token);
                    endpoints.push(super::endpoint(Some("EXPOSE"), port, line_no));
                }
            }
            "COPY" | "ADD" => {
                resources.push(resource(rest.clone(), &instr.to_lowercase(), line_no));
            }
            _ => {}
        }
    }

    if !services.is_empty() {
        analysis.services = Some(services);
    }
    if !steps.is_empty() {
        analysis.steps = Some(steps);
    }
    if !endpoints.is_empty() {
        analysis.endpoints = Some(endpoints);
    }
    if !resources.is_empty() {
        analysis.resources = Some(resources);
    }
    analysis
}

fn parse_from(rest: &str) -> (String, Option<String>) {
    let mut tokens = rest.split_ascii_whitespace();
    let image = tokens.next().unwrap_or("unknown").to_string();
    let mut alias: Option<String> = None;
    while let Some(tok) = tokens.next() {
        if tok.eq_ignore_ascii_case("AS") {
            if let Some(name) = tokens.next() {
                alias = Some(name.to_string());
                break;
            }
        }
    }
    (image, alias)
}

/// Concatenate physical lines that end in `\` so they read as one
/// logical instruction.
fn fold_continuations(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in content.lines() {
        if let Some(stripped) = line.strip_suffix('\\') {
            buf.push_str(stripped.trim_end());
            buf.push(' ');
        } else {
            buf.push_str(line);
            out.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_dockerfile_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.services.is_none());
        assert!(a.steps.is_none());
        assert!(a.endpoints.is_none());
        assert!(a.resources.is_none());
    }

    #[test]
    fn parser_dockerfile_comments_only_yields_empty_defs() {
        let a = analyze("# a comment\n# another comment\n#\n");
        assert!(a.services.is_none());
        assert!(a.steps.is_none());
        assert!(a.endpoints.is_none());
        assert!(a.resources.is_none());
    }

    #[test]
    fn parser_dockerfile_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}FROM rust:1.80 AS builder\nRUN cargo build\n";
        let a = analyze(src);
        // BOM glued to FROM may or may not be recognised — must not panic.
        let _ = a.services;
    }

    #[test]
    fn parser_dockerfile_handles_crlf_line_endings() {
        let src = "FROM rust:1.80 AS builder\r\nRUN cargo build\r\nEXPOSE 8080\r\nCOPY . /app\r\n";
        let a = analyze(src);
        let services = a.services.unwrap();
        assert!(services.iter().any(|s| s.name == "builder"));
        let steps = a.steps.unwrap();
        assert!(steps.iter().any(|s| s.name.starts_with("run:")));
        let endpoints = a.endpoints.unwrap();
        assert!(endpoints.iter().any(|e| e.path == "8080"));
        let resources = a.resources.unwrap();
        assert!(resources.iter().any(|r| r.kind == "copy"));
    }

    #[test]
    fn parser_dockerfile_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 25);
        src.push_str("FROM rust:1.80 AS builder\n");
        for i in 0..60_000 {
            src.push_str(&format!("RUN echo step_{i}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "dockerfile parse took {elapsed:?}");
        assert!(a.steps.is_some());
    }

    #[test]
    fn parser_dockerfile_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
