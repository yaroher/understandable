//! Coverage for `parse_porcelain_v2` — the byte-stream parser behind
//! `changed_files_from_git`. The parser is decoupled from the actual
//! `git status` invocation precisely so tests can feed canned NUL-
//! terminated payloads without spawning a real git subprocess.
//!
//! Pulled in via `#[path]` because `ua-cli` is a binary crate and has no
//! lib target. The included module declares `parse_porcelain_v2` as
//! `pub(crate)`, which becomes pub-visible to the test crate.

#[path = "../src/builders/diff_analyzer.rs"]
#[allow(dead_code, unused_imports)]
mod diff_analyzer;

use diff_analyzer::parse_porcelain_v2;

/// Rename arrow inside the *path itself* — porcelain v2 still parses
/// cleanly because records are NUL-terminated. The parser must emit the
/// new path (`new -> name.rs`) and *swallow* the original-path chunk
/// (`old.rs`) that follows in the next NUL slot.
#[test]
fn rename_record_emits_new_path_and_drains_original() {
    // `2 R. N... <metadata...> R100 <new>\0<orig>\0`
    let raw = "2 R. N... 100644 100644 100644 abc def R100 new -> name.rs\0old.rs\0\
               1 .M N... 100644 100644 100644 abc def src/lib.rs\0";
    let files = parse_porcelain_v2(raw);
    // The trailing ordinary record must still appear — i.e. the parser
    // didn't accidentally consume the `1 ...` record while draining the
    // rename original-path slot.
    assert_eq!(
        files,
        vec!["new -> name.rs".to_string(), "src/lib.rs".to_string()]
    );
}

/// Paths with embedded spaces — the path is the entire remainder after
/// the metadata fields, so spaces inside the path don't terminate the
/// record (NULs do).
#[test]
fn ordinary_record_with_multiple_spaces_in_path() {
    let raw = "1 .M N... 100644 100644 100644 abc def some dir/with multiple spaces.rs\0";
    let files = parse_porcelain_v2(raw);
    assert_eq!(
        files,
        vec!["some dir/with multiple spaces.rs".to_string()]
    );
}

/// Quoted paths — porcelain v2 with `-z` does NOT escape special chars
/// (that's a v1 behaviour), so the literal quote characters are part of
/// the path and must round-trip verbatim.
#[test]
fn quoted_paths_preserved_verbatim() {
    let raw = "1 .M N... 100644 100644 100644 abc def \"quoted name\".rs\0\
               ? \"another quoted\".rs\0";
    let files = parse_porcelain_v2(raw);
    assert_eq!(
        files,
        vec![
            "\"quoted name\".rs".to_string(),
            "\"another quoted\".rs".to_string(),
        ]
    );
}

/// Mixed batch covering ordinary, rename-with-arrow, untracked, and an
/// unmerged record. Demonstrates the parser's record-kind dispatch in
/// one go.
#[test]
fn mixed_batch_dispatches_correctly() {
    let raw = "# branch.oid abc\0\
               1 .M N... 100644 100644 100644 a b src/main.rs\0\
               2 R. N... 100644 100644 100644 c d R090 dst with -> arrow.rs\0src.rs\0\
               u UU N... 100644 100644 100644 100644 a b c unmerged file.rs\0\
               ? new file.rs\0\
               ! ignored.rs\0";
    let files = parse_porcelain_v2(raw);
    assert_eq!(
        files,
        vec![
            "src/main.rs".to_string(),
            "dst with -> arrow.rs".to_string(),
            "unmerged file.rs".to_string(),
            "new file.rs".to_string(),
            "ignored.rs".to_string(),
        ]
    );
}

/// Empty payload yields an empty list — guards against off-by-one
/// regressions in the iterator drain loop.
#[test]
fn empty_payload_yields_no_files() {
    assert!(parse_porcelain_v2("").is_empty());
    assert!(parse_porcelain_v2("\0\0\0").is_empty());
}
