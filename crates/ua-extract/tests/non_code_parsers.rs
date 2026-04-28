//! Integration coverage for the non-code parsers wired into
//! [`default_registry`]. These tests exercise the public surface
//! (`PluginRegistry::analyze_file`) end-to-end so a registration
//! regression in `parsers.rs` would surface here.

use ua_extract::default_registry;

#[test]
fn yaml_top_level_keys() {
    let r = default_registry();
    let src = "name: demo\nversion: 1\nservices:\n  api: {}\n";
    let a = r.analyze_file("yaml", "compose.yml", src).unwrap();
    let defs = a.definitions.expect("yaml definitions present");
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"name"));
    assert!(names.contains(&"services"));
}

#[test]
fn json_openapi_endpoints() {
    let r = default_registry();
    let src = r#"{
  "openapi": "3.0.0",
  "paths": {"/users": {"get": {}}}
}"#;
    let a = r.analyze_file("json", "openapi.json", src).unwrap();
    let endpoints = a.endpoints.expect("openapi endpoints present");
    assert!(endpoints.iter().any(|e| e.path == "/users"));
}

#[test]
fn toml_tables_and_keys() {
    let r = default_registry();
    let src = "[server]\nport = 8080\n";
    let a = r.analyze_file("toml", "Config.toml", src).unwrap();
    let defs = a.definitions.expect("toml definitions present");
    let kinds: Vec<(&str, &str)> = defs
        .iter()
        .map(|d| (d.kind.as_str(), d.name.as_str()))
        .collect();
    assert!(kinds.contains(&("section", "server")));
    assert!(kinds.contains(&("variable", "port")));
}

#[test]
fn markdown_headings() {
    let r = default_registry();
    let src = "# Top\n## Sub\n### Deep\n";
    let a = r.analyze_file("markdown", "README.md", src).unwrap();
    let sections = a.sections.expect("markdown sections present");
    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].name, "Top");
    assert_eq!(sections[2].level, 3);
}

#[test]
fn protobuf_messages_enums_services() {
    let r = default_registry();
    let src = r#"
message User { string name = 1; }
enum Role { ADMIN = 0; }
service UserService { rpc Get (User) returns (User); }
"#;
    let a = r.analyze_file("protobuf", "user.proto", src).unwrap();
    let defs = a.definitions.expect("proto definitions present");
    let kinds: std::collections::HashSet<&str> = defs.iter().map(|d| d.kind.as_str()).collect();
    assert!(kinds.contains("message"));
    assert!(kinds.contains("enum"));
    assert!(kinds.contains("service"));
}

#[test]
fn graphql_types_and_query_endpoints() {
    let r = default_registry();
    let src = r#"
type User {
  id: ID!
}
type Query {
  user(id: ID!): User
}
"#;
    let a = r.analyze_file("graphql", "schema.graphql", src).unwrap();
    let defs = a.definitions.expect("gql defs present");
    assert!(defs.iter().any(|d| d.kind == "type" && d.name == "User"));
    let endpoints = a.endpoints.expect("gql endpoints present");
    assert!(endpoints.iter().any(|e| e.path == "Query.user"));
}

#[test]
fn shell_functions_and_imports() {
    let r = default_registry();
    let src = r#"
greet() { echo hi; }
source ./lib/common.sh
"#;
    let a = r.analyze_file("shell", "tool.sh", src).unwrap();
    let defs = a.definitions.expect("shell defs present");
    assert!(defs
        .iter()
        .any(|d| d.name == "greet" && d.kind == "function"));
    assert!(a.imports.iter().any(|i| i.source == "./lib/common.sh"));
}

#[test]
fn sql_create_statements() {
    let r = default_registry();
    let src = r#"
CREATE TABLE users (id INT);
CREATE INDEX users_email_idx ON users(email);
CREATE MATERIALIZED VIEW stats AS SELECT 1;
"#;
    let a = r.analyze_file("sql", "schema.sql", src).unwrap();
    let defs = a.definitions.expect("sql defs present");
    let pairs: Vec<(&str, &str)> = defs
        .iter()
        .map(|d| (d.kind.as_str(), d.name.as_str()))
        .collect();
    assert!(pairs.contains(&("table", "users")));
    assert!(pairs.contains(&("index", "users_email_idx")));
    assert!(pairs.contains(&("view", "stats")));
}

#[test]
fn terraform_resources_and_blocks() {
    let r = default_registry();
    let src = r#"
resource "aws_s3_bucket" "logs" { bucket = "x" }
variable "region" { type = string }
output "name" { value = "x" }
data "aws_caller_identity" "current" {}
"#;
    let a = r.analyze_file("terraform", "main.tf", src).unwrap();
    let resources = a.resources.expect("terraform resources present");
    assert!(resources
        .iter()
        .any(|r| r.kind == "aws_s3_bucket" && r.name == "logs"));
    let defs = a.definitions.expect("terraform defs present");
    let pairs: Vec<(&str, &str)> = defs
        .iter()
        .map(|d| (d.kind.as_str(), d.name.as_str()))
        .collect();
    assert!(pairs.contains(&("variable", "region")));
    assert!(pairs.contains(&("output", "name")));
    assert!(pairs.contains(&("data", "aws_caller_identity.current")));
}
