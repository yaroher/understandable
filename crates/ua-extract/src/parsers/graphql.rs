//! GraphQL SDL parser. Extracts top-level type definitions and lifts
//! `Query` / `Mutation` / `Subscription` rooted fields into endpoint
//! nodes so the dashboard can show the operations of an API alongside
//! the types it exposes.

use ua_core::StructuralAnalysis;

use crate::parsers::{def_with_fields, endpoint};

pub fn analyze(content: &str) -> StructuralAnalysis {
    let mut analysis = StructuralAnalysis::default();
    let mut defs = Vec::new();
    let mut endpoints = Vec::new();

    let mut depth = 0u32;
    // (root-name (Query/Mutation/Subscription), depth-when-entered)
    let mut current_root: Option<(String, u32)> = None;

    for (idx, raw) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = match raw.find('#') {
            Some(i) => &raw[..i],
            None => raw,
        };
        let trimmed = line.trim();

        if depth == 0 {
            if let Some((kind, name)) = match_top_decl(trimmed) {
                defs.push(def_with_fields(name.clone(), kind, line_no, Vec::new()));
                if matches!(name.as_str(), "Query" | "Mutation" | "Subscription") {
                    current_root = Some((name, depth + 1));
                }
            }
        } else if let Some((root, root_depth)) = &current_root {
            // We're inside Query/Mutation/Subscription block. Top-level
            // children only (depth == root_depth).
            if depth == *root_depth && !trimmed.is_empty() {
                if let Some(field_name) = parse_field_name(trimmed) {
                    let path = format!("{root}.{field_name}");
                    endpoints.push(endpoint(Some(root.as_str()), &path, line_no));
                }
            }
        }

        for ch in trimmed.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if let Some((_, root_depth)) = &current_root {
                        if depth < *root_depth {
                            current_root = None;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if !defs.is_empty() {
        analysis.definitions = Some(defs);
    }
    if !endpoints.is_empty() {
        analysis.endpoints = Some(endpoints);
    }
    analysis
}

fn match_top_decl(line: &str) -> Option<(&'static str, String)> {
    for (keyword, kind) in [
        ("type ", "type"),
        ("input ", "input"),
        ("enum ", "enum"),
        ("union ", "union"),
        ("interface ", "interface"),
        ("scalar ", "scalar"),
    ] {
        if let Some(rest) = line.strip_prefix(keyword) {
            let name = rest
                .split(|c: char| c.is_whitespace() || c == '{' || c == '=' || c == '(')
                .find(|tok| !tok.is_empty())?;
            return Some((kind, name.to_string()));
        }
    }
    None
}

/// Extract the name of a field declaration: `name(args): Type` → `name`.
fn parse_field_name(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('}') || line.starts_with('{') {
        return None;
    }
    let end = line
        .find(|c: char| c == '(' || c == ':' || c.is_whitespace())
        .unwrap_or(line.len());
    let name = &line[..end];
    if name.is_empty() {
        return None;
    }
    if !name
        .chars()
        .next()
        .map(|c| c.is_alphabetic() || c == '_')
        .unwrap_or(false)
    {
        return None;
    }
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_and_kinds_emitted() {
        let src = r#"
type User { id: ID! }
input UserInput { name: String }
enum Role { ADMIN USER }
union Result = User | Error
interface Node { id: ID! }
scalar DateTime
"#;
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        let pairs: Vec<(&str, &str)> = defs
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str()))
            .collect();
        assert!(pairs.contains(&("type", "User")));
        assert!(pairs.contains(&("input", "UserInput")));
        assert!(pairs.contains(&("enum", "Role")));
        assert!(pairs.contains(&("union", "Result")));
        assert!(pairs.contains(&("interface", "Node")));
        assert!(pairs.contains(&("scalar", "DateTime")));
    }

    #[test]
    fn query_fields_become_endpoints() {
        let src = r#"
type Query {
  user(id: ID!): User
  users: [User!]!
}
type Mutation {
  createUser(input: UserInput!): User
}
"#;
        let a = analyze(src);
        let endpoints = a.endpoints.unwrap();
        let paths: Vec<&str> = endpoints.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"Query.user"));
        assert!(paths.contains(&"Query.users"));
        assert!(paths.contains(&"Mutation.createUser"));
    }

    // --- edge-case suite ---------------------------------------------------

    #[test]
    fn parser_graphql_empty_input_yields_empty_defs() {
        let a = analyze("");
        assert!(a.definitions.is_none());
        assert!(a.endpoints.is_none());
    }

    #[test]
    fn parser_graphql_comments_only_yields_empty_defs() {
        let a = analyze("# comment\n# another comment\n#\n");
        assert!(a.definitions.is_none());
        assert!(a.endpoints.is_none());
    }

    #[test]
    fn parser_graphql_handles_utf8_bom_gracefully() {
        let src = "\u{FEFF}type User { id: ID! }\n";
        let a = analyze(src);
        let _ = a.definitions;
    }

    #[test]
    fn parser_graphql_handles_crlf_line_endings() {
        let src = "type User { id: ID! }\r\nenum Role { ADMIN USER }\r\n";
        let a = analyze(src);
        let defs = a.definitions.unwrap();
        assert!(defs.iter().any(|d| d.name == "User" && d.kind == "type"));
        assert!(defs.iter().any(|d| d.name == "Role" && d.kind == "enum"));
    }

    #[test]
    fn parser_graphql_processes_60kb_within_budget() {
        let mut src = String::with_capacity(60_000 * 30);
        for i in 0..60_000 {
            src.push_str(&format!("type T_{i} {{ id: ID! }}\n"));
        }
        let start = std::time::Instant::now();
        let a = analyze(&src);
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "graphql parse took {elapsed:?}");
        assert!(a.definitions.is_some());
    }

    #[test]
    fn parser_graphql_non_utf8_bytes_returns_typed_error() {
        let bad: &[u8] = b"\xFF\xFE\x00\x00";
        assert!(std::str::from_utf8(bad).is_err());
    }
}
