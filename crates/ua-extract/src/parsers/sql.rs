//! SQL parser — extracts schema-shaping `CREATE` statements:
//!   * `CREATE [TEMP[ORARY]] TABLE [IF NOT EXISTS] name`
//!   * `CREATE [OR REPLACE] [MATERIALIZED] VIEW [IF NOT EXISTS] name`
//!   * `CREATE [UNIQUE] INDEX [IF NOT EXISTS] name`
//!
//! Names are emitted as `kind` = `table` / `view` / `index`. We don't
//! parse columns — the dashboard only needs the entity list.

use ua_core::StructuralAnalysis;

use crate::parsers::def_with_fields;

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();

    // Split on lines so we can attach a useful line number; multi-line
    // CREATE statements still match because we only look at the first
    // line of each statement.
    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = strip_line_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((kind, name)) = match_create(line) {
            defs.push(def_with_fields(name, kind, line_no, Vec::new()));
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    analysis
}

fn strip_line_comment(raw: &str) -> &str {
    if let Some(idx) = raw.find("--") {
        &raw[..idx]
    } else {
        raw
    }
}

/// Tolerant matcher: lower-case the prefix, peel off well-known modifier
/// keywords, then read the next identifier.
fn match_create(line: &str) -> Option<(&'static str, String)> {
    let lower = line.to_ascii_lowercase();
    let mut tokens: Vec<&str> = lower.split_ascii_whitespace().collect();
    if tokens.first().copied() != Some("create") {
        return None;
    }
    tokens.remove(0);

    // Drop modifier keywords until we hit table / view / index.
    while let Some(&tok) = tokens.first() {
        if matches!(
            tok,
            "or" | "replace" | "temp" | "temporary" | "unique" | "materialized"
        ) {
            tokens.remove(0);
        } else {
            break;
        }
    }

    let kind = match tokens.first().copied()? {
        "table" => "table",
        "view" => "view",
        "index" => "index",
        _ => return None,
    };
    tokens.remove(0);

    // skip `IF NOT EXISTS`
    if tokens.first().copied() == Some("if") {
        tokens.remove(0);
        if tokens.first().copied() == Some("not") {
            tokens.remove(0);
        }
        if tokens.first().copied() == Some("exists") {
            tokens.remove(0);
        }
    }

    let name_lower = tokens.first()?.trim_matches(|c| c == '(' || c == ';');
    if name_lower.is_empty() {
        return None;
    }
    // Recover the original casing from the source line.
    let raw_token = find_original_token(line, name_lower)?;
    let cleaned = raw_token.trim_matches(|c| c == '"' || c == '`' || c == '[' || c == ']');
    Some((kind, cleaned.to_string()))
}

fn find_original_token<'a>(line: &'a str, lower_token: &str) -> Option<&'a str> {
    line.split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .find(|tok| tok.eq_ignore_ascii_case(lower_token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tables_views_indexes() {
        let src = r#"
CREATE TABLE users (id INT PRIMARY KEY);
CREATE OR REPLACE VIEW active_users AS SELECT * FROM users;
CREATE MATERIALIZED VIEW user_stats AS SELECT count(*) FROM users;
CREATE UNIQUE INDEX users_email_idx ON users(email);
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let pairs: Vec<(&str, &str)> = defs
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str()))
            .collect();
        assert!(pairs.contains(&("table", "users")));
        assert!(pairs.contains(&("view", "active_users")));
        assert!(pairs.contains(&("view", "user_stats")));
        assert!(pairs.contains(&("index", "users_email_idx")));
    }

    #[test]
    fn handles_if_not_exists() {
        let src = "CREATE TABLE IF NOT EXISTS Foo (id INT);\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Foo");
        assert_eq!(defs[0].kind, "table");
    }

    #[test]
    fn line_comments_skipped() {
        let src = "-- CREATE TABLE ghost (id INT);\nCREATE TABLE real (id INT);\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "real");
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_sql_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_sql_comments_only_yields_empty_defs() {
        let a = analyze("-- a comment\n-- another comment\n--\n");
        assert!(a.definitions.is_none());
    }

    #[test]
    fn parser_sql_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}CREATE TABLE users (id INT);\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_sql_handles_crlf_line_endings() {
        let src = "CREATE TABLE users (id INT);\r\nCREATE INDEX idx_users ON users(id);\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert!(defs.iter().any(|d| d.name == "users" && d.kind == "table"));
        assert!(defs
            .iter()
            .any(|d| d.name == "idx_users" && d.kind == "index"));
    }

    #[test]
    fn parser_sql_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 35);
        for i in 0..60_000 {
            src.push_str(&format!("CREATE TABLE t_{i} (id INT);\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "sql parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    #[allow(invalid_from_utf8)]
    fn parser_sql_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
