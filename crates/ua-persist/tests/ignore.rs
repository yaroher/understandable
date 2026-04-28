use std::fs;
use std::path::PathBuf;

use ua_persist::{walk_project, IgnoreFilter};

fn touch(p: &PathBuf, content: &str) {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

#[test]
fn walk_respects_gitignore_and_understandignore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Project tree.
    touch(&root.join("src/main.rs"), "");
    touch(&root.join("src/lib.rs"), "");
    touch(&root.join("target/build.log"), "");
    touch(&root.join("dist/bundle.js"), "");
    touch(&root.join(".gitignore"), "target/\n");
    touch(&root.join(".understandignore"), "dist/\n");

    let filter = IgnoreFilter::default();
    let paths: Vec<String> = walk_project(root, &filter)
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(paths.iter().any(|p| p.ends_with("src/main.rs")));
    assert!(paths.iter().any(|p| p.ends_with("src/lib.rs")));
    // gitignored
    assert!(!paths.iter().any(|p| p.contains("target/")));
    // understandignored
    assert!(!paths.iter().any(|p| p.contains("dist/")));
}

/// `IgnoreFilter::extra_ignore_paths` is the YAML-driven equivalent of
/// stuffing entries into `.understandignore`. The walker must prune
/// matching subtrees natively (no post-walk filter), and entries with
/// or without trailing slashes must behave identically.
#[test]
fn walk_honours_extra_ignore_paths_from_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    touch(&root.join("src/main.rs"), "");
    touch(&root.join("src/lib.rs"), "");
    touch(&root.join("vendor/third_party/code.rs"), "");
    touch(&root.join("generated/proto.rs"), "");
    touch(&root.join("docs/readme.md"), "");

    let filter = IgnoreFilter {
        // Trailing slash on `vendor/`, none on `generated` — both forms
        // must work.
        extra_ignore_paths: vec!["vendor/".into(), "generated".into()],
        ..IgnoreFilter::default()
    };

    let paths: Vec<String> = walk_project(root, &filter)
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
        .collect();

    // Source files survive.
    assert!(paths.iter().any(|p| p.ends_with("src/main.rs")));
    assert!(paths.iter().any(|p| p.ends_with("src/lib.rs")));
    assert!(paths.iter().any(|p| p.ends_with("docs/readme.md")));

    // Extra ignored subtrees pruned.
    assert!(
        !paths.iter().any(|p| p.contains("vendor/")),
        "vendor/ entries leaked: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains("generated/")),
        "generated/ entries leaked: {paths:?}"
    );
}

/// Empty / whitespace-only entries in `extra_ignore_paths` must be
/// tolerated — users sometimes leave a stray blank line in their YAML
/// list. Behaviour should match the no-extras case exactly.
#[test]
fn walk_tolerates_blank_extra_ignore_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("src/main.rs"), "");

    let filter = IgnoreFilter {
        extra_ignore_paths: vec!["".into(), "   ".into()],
        ..IgnoreFilter::default()
    };
    let paths: Vec<String> = walk_project(root, &filter)
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(paths.iter().any(|p| p.ends_with("src/main.rs")));
}
