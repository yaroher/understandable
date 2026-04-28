use ua_extract::default_registry;

#[test]
fn dockerfile_parsed_into_services_and_steps() {
    let src = r#"FROM rust:1.83 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/understandable /usr/local/bin/understandable
EXPOSE 8080/tcp
CMD ["understandable"]
"#;
    let r = default_registry();
    let a = r.analyze_file("dockerfile", "Dockerfile", src).unwrap();
    let services = a.services.unwrap();
    assert_eq!(services.len(), 2);
    assert_eq!(services[0].image.as_deref(), Some("rust:1.83"));
    assert_eq!(services[0].name, "builder");
    assert_eq!(services[1].image.as_deref(), Some("debian:bookworm-slim"));
    let steps = a.steps.unwrap();
    assert!(steps.iter().any(|s| s.name.starts_with("run:")));
    assert!(steps.iter().any(|s| s.name.starts_with("cmd:")));
    let endpoints = a.endpoints.unwrap();
    assert_eq!(endpoints[0].path, "8080");
    assert_eq!(endpoints[0].method.as_deref(), Some("EXPOSE"));
}

#[test]
fn makefile_parsed_into_steps_and_definitions() {
    let src = r#"
CC := gcc
CFLAGS = -Wall

all: build test

build:
	$(CC) main.c -o main

test: build
	./main --check
"#;
    let r = default_registry();
    let a = r.analyze_file("makefile", "Makefile", src).unwrap();
    let steps = a.steps.unwrap();
    let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
    assert!(names.iter().any(|n| n.starts_with("all <- build test")));
    assert!(names.iter().any(|n| n.starts_with("build")));
    assert!(names.iter().any(|n| n.starts_with("test <- build")));
    let defs = a.definitions.unwrap();
    let dnames: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(dnames.contains(&"CC"));
    assert!(dnames.contains(&"CFLAGS"));
}

#[test]
fn env_file_parsed() {
    let src = r#"# database
DB_URL="postgres://localhost/foo"
DB_POOL=5
export API_KEY='secret-value'  # inline note
EMPTY=
"#;
    let r = default_registry();
    let a = r.analyze_file("env", ".env", src).unwrap();
    let defs = a.definitions.unwrap();
    let by_name: std::collections::HashMap<&str, &str> = defs
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.fields.first().map(|s| s.as_str()).unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        by_name.get("DB_URL").copied(),
        Some("postgres://localhost/foo")
    );
    assert_eq!(by_name.get("DB_POOL").copied(), Some("5"));
    assert_eq!(by_name.get("API_KEY").copied(), Some("secret-value"));
    assert_eq!(by_name.get("EMPTY").copied(), Some(""));
}

#[test]
fn ini_file_parsed_into_section_aware_defs() {
    let src = r#"
; config
[server]
host = 127.0.0.1
port = 8080

[database]
url = sqlite:///data.db
"#;
    let r = default_registry();
    let a = r.analyze_file("ini", "config.ini", src).unwrap();
    let defs = a.definitions.unwrap();
    let sections: Vec<&str> = defs
        .iter()
        .filter(|d| d.kind == "section")
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(sections, vec!["server", "database"]);
    let host = defs.iter().find(|d| d.name == "host").unwrap();
    assert_eq!(
        host.fields,
        vec!["server".to_string(), "127.0.0.1".to_string()]
    );
    let url = defs.iter().find(|d| d.name == "url").unwrap();
    assert_eq!(
        url.fields,
        vec!["database".to_string(), "sqlite:///data.db".to_string()]
    );
}
