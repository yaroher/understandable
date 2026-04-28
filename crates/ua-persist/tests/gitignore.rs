//! Integration tests for the gitignore managed-block manager.
//!
//! These complement the inline `gitignore::tests` in `src/gitignore.rs`
//! by exercising the multi-block / duplicate-cleanup contract specified
//! in the task: after `apply_block` the file must contain exactly one
//! managed block; stale duplicates are scrubbed; idempotence is byte-
//! exact.

use std::fs;

use ua_core::ProjectSettings;
use ua_persist::{apply_block, render_block, GitignoreOutcome, GitignorePolicy};

const HEADER: &str = "# >>> understandable >>>";
const FOOTER: &str = "# <<< understandable <<<";

fn settings(commit_db: bool, dir: &str) -> ProjectSettings {
    let mut s = ProjectSettings::recommended();
    s.git.commit_db = commit_db;
    s.storage.dir = dir.into();
    s
}

fn count_blocks(s: &str) -> usize {
    // header occurrences are a lower bound; footer occurrences a paired
    // upper bound. They should match — assert that to catch malformed
    // output explicitly.
    let h = s.matches(HEADER).count();
    let f = s.matches(FOOTER).count();
    assert_eq!(h, f, "unbalanced markers in:\n{s}");
    h
}

#[test]
fn warning_line_is_rendered() {
    let s = settings(true, ".understandable");
    let block = render_block(GitignorePolicy::CommitDb, &s.storage);
    assert!(
        block.contains("any edits here will be overwritten on next `understandable init`"),
        "block missing warning line: {block}"
    );
}

#[test]
fn two_stale_blocks_collapse_to_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    // User content + two stale managed blocks (different / outdated bodies).
    let initial = "\
node_modules/
.idea/
# >>> understandable >>>
# stale-1
.old-storage/
# <<< understandable <<<

src/keep_me.txt
# >>> understandable >>>
# stale-2
.older-storage/
# <<< understandable <<<
";
    fs::write(&path, initial).unwrap();
    assert_eq!(count_blocks(initial), 2);

    let s = settings(true, ".understandable");
    let outcome = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(outcome, GitignoreOutcome::Updated);

    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(
        count_blocks(&body),
        1,
        "should be exactly one block:\n{body}"
    );
    // User lines preserved.
    assert!(body.contains("node_modules/"));
    assert!(body.contains(".idea/"));
    assert!(body.contains("src/keep_me.txt"));
    // Stale bodies gone.
    assert!(!body.contains("stale-1"));
    assert!(!body.contains("stale-2"));
    assert!(!body.contains(".old-storage"));
    assert!(!body.contains(".older-storage"));
    // Fresh content present.
    assert!(body.contains(".understandable/intermediate/"));
    assert!(body.contains(".understandable/tmp/"));
}

#[test]
fn three_stale_blocks_collapse_to_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    let initial = "\
# user header
.env
# >>> understandable >>>
# block-A
# <<< understandable <<<
gap-line-1
# >>> understandable >>>
# block-B
# <<< understandable <<<
gap-line-2
# >>> understandable >>>
# block-C
# <<< understandable <<<
trailing-user-line
";
    fs::write(&path, initial).unwrap();
    assert_eq!(count_blocks(initial), 3);

    let s = settings(false, ".understandable");
    let outcome = apply_block(dir.path(), GitignorePolicy::IgnoreAll, &s.storage).unwrap();
    assert_eq!(outcome, GitignoreOutcome::Updated);

    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(count_blocks(&body), 1);
    // User lines and gaps preserved.
    assert!(body.contains(".env"));
    assert!(body.contains("gap-line-1"));
    assert!(body.contains("gap-line-2"));
    assert!(body.contains("trailing-user-line"));
    // Stale bodies gone.
    assert!(!body.contains("block-A"));
    assert!(!body.contains("block-B"));
    assert!(!body.contains("block-C"));
    // Fresh ignore-all content.
    assert!(body.contains(".understandable/"));
}

#[test]
fn idempotent_when_already_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    fs::write(&path, "node_modules/\n").unwrap();

    let s = settings(true, ".understandable");
    let first = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(first, GitignoreOutcome::Appended);
    let after_first = fs::read_to_string(&path).unwrap();

    let second = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(second, GitignoreOutcome::AlreadyCurrent);
    let after_second = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after_first, after_second,
        "byte-identical on idempotent apply"
    );
}

#[test]
fn idempotent_after_collapsing_duplicates() {
    // Apply twice when the input had duplicates: the second call should
    // be AlreadyCurrent because the first call already collapsed.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    let initial = "\
node_modules/
# >>> understandable >>>
# stale
# <<< understandable <<<
# >>> understandable >>>
# stale
# <<< understandable <<<
";
    fs::write(&path, initial).unwrap();

    let s = settings(true, ".understandable");
    let first = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(first, GitignoreOutcome::Updated);
    let after_first = fs::read_to_string(&path).unwrap();
    assert_eq!(count_blocks(&after_first), 1);

    let second = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(second, GitignoreOutcome::AlreadyCurrent);
    let after_second = fs::read_to_string(&path).unwrap();
    assert_eq!(after_first, after_second);
}

#[test]
fn block_at_end_without_trailing_newline_before_marker() {
    // User content ends without a trailing newline, then the managed
    // block follows immediately. Locating + replacing must still work.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    // Note: NO `\n` after `node_modules/` before the header.
    let initial = "node_modules/# >>> understandable >>>\n# stale\n# <<< understandable <<<\n";
    fs::write(&path, initial).unwrap();
    assert_eq!(count_blocks(initial), 1);

    let s = settings(true, ".understandable");
    let outcome = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(outcome, GitignoreOutcome::Updated);

    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(count_blocks(&body), 1);
    assert!(body.contains("node_modules/"));
    assert!(!body.contains("# stale"));
    assert!(body.contains(".understandable/intermediate/"));
}

#[test]
fn block_at_end_of_file_no_trailing_newline_after_footer() {
    // The managed block sits at end-of-file with no trailing `\n` after
    // the footer. After a re-apply we should still get a single block
    // and a clean replacement (the renderer always adds a trailing
    // newline on the new block).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    // No `\n` after the FOOTER.
    let initial = "node_modules/\n# >>> understandable >>>\n# stale\n# <<< understandable <<<";
    fs::write(&path, initial).unwrap();
    assert_eq!(count_blocks(initial), 1);

    let s = settings(true, ".understandable");
    let outcome = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    assert_eq!(outcome, GitignoreOutcome::Updated);

    let body = fs::read_to_string(&path).unwrap();
    assert_eq!(count_blocks(&body), 1);
    assert!(body.contains("node_modules/"));
    assert!(!body.contains("# stale"));
    assert!(body.contains(".understandable/intermediate/"));
}

#[test]
fn first_block_position_preserved() {
    // The new block should land where the FIRST original block was, not
    // get shoved to the end.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    let initial = "\
TOP_USER_LINE
# >>> understandable >>>
# old
# <<< understandable <<<
MIDDLE_USER_LINE
# >>> understandable >>>
# old2
# <<< understandable <<<
BOTTOM_USER_LINE
";
    fs::write(&path, initial).unwrap();

    let s = settings(true, ".understandable");
    apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
    let body = fs::read_to_string(&path).unwrap();

    let header_pos = body.find(HEADER).unwrap();
    let top_pos = body.find("TOP_USER_LINE").unwrap();
    let middle_pos = body.find("MIDDLE_USER_LINE").unwrap();
    let bottom_pos = body.find("BOTTOM_USER_LINE").unwrap();

    assert!(
        top_pos < header_pos,
        "fresh block should follow TOP_USER_LINE"
    );
    assert!(
        header_pos < middle_pos,
        "fresh block should sit before MIDDLE_USER_LINE (where the first stale block lived)"
    );
    assert!(middle_pos < bottom_pos);
}
