//! Prompt builders for the standalone enrichment path.
//!
//! These return the same kind of system + user pair the markdown
//! `file-analyzer` agent produces, so the binary's standalone mode and
//! the IDE-driven mode generate comparable summaries.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileSummaryRequest<'a> {
    pub project_name: &'a str,
    pub language: &'a str,
    pub path: &'a str,
    pub content: &'a str,
}

/// `(system, user)` prompts for a `file` node. The expected response
/// is JSON parseable into [`FileSummaryResponse`].
pub fn file_summary_prompts(req: &FileSummaryRequest<'_>) -> (String, String) {
    let system = "You annotate source files for a codebase knowledge graph. \
Reply with JSON only — no prose, no markdown fences. Schema: \
{\"summary\":string,\"tags\":string[],\"complexity\":\"simple|moderate|complex\",\"languageNotes\":string}.\n\
- summary: 1-2 sentence purpose statement.\n\
- tags: 3-6 lowercase keywords describing the file's role.\n\
- complexity: simple = boilerplate/config; moderate = ordinary feature code; complex = heavy logic, performance work, or hard concurrency.\n\
- languageNotes: optional short note on language idioms worth pointing out (closures, generics, lifetimes, etc.). Empty string if nothing notable."
        .to_string();
    let user = format!(
        "Project: {project}\nLanguage: {lang}\nPath: {path}\n\n```{lang}\n{content}\n```\n\nReturn the JSON.",
        project = req.project_name,
        lang = req.language,
        path = req.path,
        content = trim_for_prompt(req.content, 8_000),
    );
    (system, user)
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileSummaryResponse {
    pub summary: String,
    pub tags: Vec<String>,
    #[serde(default = "default_complexity")]
    pub complexity: String,
    #[serde(default)]
    pub language_notes: Option<String>,
}

fn default_complexity() -> String {
    "moderate".into()
}

/// Tolerantly parse the LLM's JSON reply: strips markdown code fences
/// before deserialising.
pub fn parse_file_summary(raw: &str) -> Result<FileSummaryResponse, serde_json::Error> {
    let cleaned = strip_code_fence(raw);
    serde_json::from_str(cleaned)
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim_start_matches('\n').trim_end_matches("```").trim();
    }
    s
}

fn trim_for_prompt(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let mut out: String = content.chars().take(max_chars).collect();
    out.push_str("\n... (truncated)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_json() {
        let raw = r#"{"summary":"x","tags":["a"],"complexity":"simple","languageNotes":"n"}"#;
        let parsed = parse_file_summary(raw).unwrap();
        assert_eq!(parsed.summary, "x");
        assert_eq!(parsed.complexity, "simple");
        assert_eq!(parsed.language_notes.as_deref(), Some("n"));
    }

    #[test]
    fn parse_fenced_json() {
        let raw = "```json\n{\"summary\":\"y\",\"tags\":[],\"complexity\":\"complex\"}\n```";
        let parsed = parse_file_summary(raw).unwrap();
        assert_eq!(parsed.summary, "y");
        assert!(parsed.language_notes.is_none());
    }
}
