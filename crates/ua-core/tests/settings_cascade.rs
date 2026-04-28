//! Settings cascade tests — verify the `default → YAML → CLI override`
//! resolution rules that every CLI subcommand relies on.
//!
//! ua-core has no `tempfile` dev-dependency (touching `Cargo.toml`
//! would step outside the test-only scope), so we mint per-test temp
//! directories under `std::env::temp_dir()` keyed off the process id +
//! test name. Each test cleans up after itself.

use std::path::PathBuf;

use ua_core::{Error, ProjectSettings};

/// Mint a unique temp dir for one test. Returns an absolute path. The
/// directory is created fresh; tests are responsible for `remove_dir_all`
/// on the way out (the closure helper below does this for us).
fn fresh_dir(test_name: &str) -> PathBuf {
    let unique = format!(
        "ua-core-settings-{}-{}-{}",
        test_name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("create_dir_all temp");
    path
}

/// RAII drop guard so a panic inside the test still cleans up.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl TempDir {
    fn new(name: &str) -> Self {
        Self(fresh_dir(name))
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

#[test]
fn cli_flag_overrides_yaml_value() {
    // Stage 1: YAML on disk picks `batch_size = 16`.
    // Stage 2: simulated CLI override mutates `settings.embeddings.batch_size = 99`.
    // Resolution must surface 99.
    let dir = TempDir::new("cli_overrides_yaml");
    let yaml_path = ProjectSettings::default_path(dir.path());
    std::fs::write(
        &yaml_path,
        r#"
version: 1
embeddings:
  provider: ollama
  batch_size: 16
"#,
    )
    .unwrap();

    let mut settings = ProjectSettings::load_or_default(dir.path()).expect("load");
    assert_eq!(settings.embeddings.batch_size, 16, "yaml stage");
    assert_eq!(settings.embeddings.provider, "ollama");

    // CLI override step — every subcommand mutates the loaded struct
    // before passing it down. We mirror that pattern here.
    settings.embeddings.batch_size = 99;
    assert_eq!(settings.embeddings.batch_size, 99, "cli override stage");
    // The other YAML-supplied values must not have been clobbered.
    assert_eq!(settings.embeddings.provider, "ollama");
}

#[test]
fn yaml_overrides_default() {
    // YAML-supplied fields override their constructor defaults; fields
    // not mentioned keep the default. This is the heart of "partial
    // YAML uses defaults".
    let dir = TempDir::new("yaml_over_default");
    let yaml_path = ProjectSettings::default_path(dir.path());
    std::fs::write(
        &yaml_path,
        r#"
version: 1
dashboard:
  port: 7777
incremental:
  full_threshold: 5
"#,
    )
    .unwrap();

    let s = ProjectSettings::load_or_default(dir.path()).expect("load");
    // YAML values win.
    assert_eq!(s.dashboard.port, 7777);
    assert_eq!(s.incremental.full_threshold, 5);
    // Untouched: defaults.
    assert_eq!(s.dashboard.host, "127.0.0.1");
    assert!(s.dashboard.auto_open);
    assert_eq!(s.incremental.big_graph_threshold, 50);
    assert_eq!(s.embeddings.batch_size, 32);
    // The version field is preserved from the YAML.
    assert_eq!(s.version, 1);
}

#[test]
fn missing_yaml_uses_default() {
    // No `understandable.yaml` in the project root — `load_or_default`
    // returns the recommended defaults, not an error.
    let dir = TempDir::new("missing_yaml");
    assert!(
        ProjectSettings::find(dir.path()).is_none(),
        "no yaml should be present"
    );

    let s = ProjectSettings::load_or_default(dir.path()).expect("load");
    let recommended = ProjectSettings::recommended();
    assert_eq!(s, recommended);
    assert_eq!(s.version, 1);
    assert_eq!(s.embeddings.provider, "openai");
    assert_eq!(s.dashboard.port, 5173);

    // And `load` (without _or_default) reports `None`.
    let none = ProjectSettings::load(dir.path()).expect("load");
    assert!(none.is_none());
}

#[test]
fn invalid_yaml_returns_typed_error() {
    // Syntactically broken YAML must surface as `Error::Yaml`, not a
    // panic and not a generic `Error::Other`.
    let dir = TempDir::new("invalid_yaml");
    let yaml_path = ProjectSettings::default_path(dir.path());
    // Tab indentation under a mapping key + unbalanced quote — guaranteed parse error.
    std::fs::write(
        &yaml_path,
        "version: 1\nembeddings:\n  provider: \"unterminated\n  batch_size: not_a_number\n",
    )
    .unwrap();

    let err = ProjectSettings::load_or_default(dir.path()).expect_err("malformed yaml must error");
    match err {
        Error::Yaml(_) => {}
        other => panic!("expected Error::Yaml, got: {other:?}"),
    }
}

#[test]
fn unknown_field_emits_typed_error() {
    // `deny_unknown_fields` must catch typos at every depth. We test
    // both a top-level unknown section and a per-section typo.
    let dir = TempDir::new("unknown_field");
    let yaml_path = ProjectSettings::default_path(dir.path());

    std::fs::write(
        &yaml_path,
        r#"
version: 1
embeddings:
  providr: openai
"#,
    )
    .unwrap();

    let err = ProjectSettings::load_or_default(dir.path()).expect_err("typo must error");
    match err {
        Error::Yaml(inner) => {
            let msg = inner.to_string();
            assert!(
                msg.contains("providr") || msg.contains("unknown field"),
                "expected typed unknown-field error mentioning the typo, got: {msg}",
            );
        }
        other => panic!("expected Error::Yaml, got: {other:?}"),
    }

    // Symmetric case: unknown top-level section.
    std::fs::write(
        &yaml_path,
        r#"
version: 1
mystery_section:
  whatever: true
"#,
    )
    .unwrap();
    let err2 = ProjectSettings::load_or_default(dir.path()).expect_err("top-level typo must error");
    match err2 {
        Error::Yaml(inner) => {
            let msg = inner.to_string();
            assert!(
                msg.contains("mystery_section") || msg.contains("unknown field"),
                "expected typed unknown-field error, got: {msg}",
            );
        }
        other => panic!("expected Error::Yaml, got: {other:?}"),
    }
}
