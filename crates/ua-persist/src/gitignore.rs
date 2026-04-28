//! Maintain a managed block inside `<project>/.gitignore`.
//!
//! Two policies depending on `git.commit_db` in the project settings:
//!
//! * **commit-DB mode (default)** — keep the `.tar.zst` files tracked,
//!   only ignore the agent's intermediate / tmp folders.
//! * **don't-commit-DB mode** — ignore the whole storage directory.
//!
//! The block is delimited by `# >>> understandable >>>` /
//! `# <<< understandable <<<` markers so we can rewrite it without
//! touching anything the user added by hand.

use std::path::Path;

use ua_core::{ProjectSettings, StorageSettings};

const HEADER: &str = "# >>> understandable >>>";
const FOOTER: &str = "# <<< understandable <<<";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitignorePolicy {
    /// Commit the DB; only ignore caches that are unsafe to ship.
    CommitDb,
    /// Do not commit the DB; ignore the whole storage directory.
    IgnoreAll,
}

impl GitignorePolicy {
    pub fn from_settings(s: &ProjectSettings) -> Self {
        if s.git.commit_db {
            Self::CommitDb
        } else {
            Self::IgnoreAll
        }
    }
}

/// Render the managed block (between the markers) for a given policy
/// and storage section.
///
/// The body always carries a "any edits here will be overwritten" line
/// so users who try to hand-tweak the block know it's volatile.
pub fn render_block(policy: GitignorePolicy, storage: &StorageSettings) -> String {
    let dir = strip_trailing_slash(&storage.dir);
    let warning = "# any edits here will be overwritten on next `understandable init`";
    let body = match policy {
        GitignorePolicy::CommitDb => format!(
            "# managed by `understandable init` — leave the DB tracked.\n{warning}\n{dir}/intermediate/\n{dir}/tmp/\n"
        ),
        GitignorePolicy::IgnoreAll => format!(
            "# managed by `understandable init` — DB stays out of git.\n{warning}\n{dir}/\n"
        ),
    };
    format!("{HEADER}\n{body}{FOOTER}\n")
}

fn strip_trailing_slash(s: &str) -> &str {
    s.strip_suffix('/').unwrap_or(s)
}

/// Result of [`apply_block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitignoreOutcome {
    /// File created from scratch.
    Created,
    /// Existing managed block replaced.
    Updated,
    /// Managed block appended to the existing file.
    Appended,
    /// File already had the same managed block — nothing changed.
    AlreadyCurrent,
}

/// Insert / replace the managed block in `<project>/.gitignore`.
///
/// Contract: after this call the file contains **exactly one** managed
/// block. If the file had multiple stale blocks (duplicates from a
/// botched merge or a hand-copied gitignore), every one of them is
/// removed and a single fresh block is written at the position of the
/// FIRST original block. If no managed block existed, the fresh one is
/// appended after the user's content.
pub fn apply_block(
    project_root: impl AsRef<Path>,
    policy: GitignorePolicy,
    storage: &StorageSettings,
) -> std::io::Result<GitignoreOutcome> {
    let path = project_root.as_ref().join(".gitignore");
    let block = render_block(policy, storage);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };

    let had_block = existing
        .as_deref()
        .map(|s| locate_block(s).is_some())
        .unwrap_or(false);

    let new_contents = match existing.as_deref() {
        None => block.clone(),
        Some(current) => {
            let blocks = locate_all_blocks(current);
            if blocks.is_empty() {
                let mut out = current.to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(&block);
                out
            } else {
                // Splice: keep `current[..first.start]`, insert the new
                // block, then walk every gap between blocks and append
                // it (skipping the block bodies themselves), then the
                // tail after the last block.
                let mut out = String::with_capacity(current.len() + block.len());
                let first_start = blocks[0].0;
                out.push_str(&current[..first_start]);
                out.push_str(&block);
                let mut cursor = blocks[0].1;
                for &(s, e) in &blocks[1..] {
                    out.push_str(&current[cursor..s]);
                    cursor = e;
                }
                out.push_str(&current[cursor..]);
                out
            }
        }
    };

    if let Some(current) = existing.as_deref() {
        if new_contents == current {
            return Ok(GitignoreOutcome::AlreadyCurrent);
        }
    }

    let outcome = match (&existing, had_block) {
        (None, _) => GitignoreOutcome::Created,
        (Some(_), true) => GitignoreOutcome::Updated,
        (Some(_), false) => GitignoreOutcome::Appended,
    };
    std::fs::write(&path, new_contents)?;
    Ok(outcome)
}

/// Find the existing managed block by header / footer markers. Returns
/// `(start_byte, end_byte_exclusive)` so callers can splice a replacement
/// in. The returned range covers the marker lines themselves and the
/// trailing newline after the footer (if any).
///
/// This now returns the FIRST block; use [`locate_all_blocks`] when you
/// need to clean up duplicates.
fn locate_block(text: &str) -> Option<(usize, usize)> {
    locate_block_from(text, 0)
}

/// Find a managed block starting at or after `from`. Same return shape
/// as [`locate_block`] — `(start, end_exclusive)` covering the marker
/// lines and the trailing newline if present.
fn locate_block_from(text: &str, from: usize) -> Option<(usize, usize)> {
    let header_rel = text[from..].find(HEADER)?;
    let header_idx = from + header_rel;
    let after_header = header_idx + HEADER.len();
    let footer_rel = text[after_header..].find(FOOTER)?;
    let footer_abs = after_header + footer_rel;
    let mut end = footer_abs + FOOTER.len();
    if text[end..].starts_with('\n') {
        end += 1;
    }
    Some((header_idx, end))
}

/// Find every managed block in `text`. Returns ranges in source order;
/// guarantees non-overlapping, sorted output. Used by [`apply_block`]
/// to scrub duplicate stale blocks left behind by manual edits / merges.
fn locate_all_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some((s, e)) = locate_block_from(text, from) {
        out.push((s, e));
        from = e;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ua_core::ProjectSettings;

    fn settings(commit_db: bool, dir: &str) -> ProjectSettings {
        let mut s = ProjectSettings::recommended();
        s.git.commit_db = commit_db;
        s.storage.dir = dir.into();
        s
    }

    #[test]
    fn render_commit_db_keeps_tracked() {
        let s = settings(true, ".understandable");
        let block = render_block(GitignorePolicy::from_settings(&s), &s.storage);
        assert!(block.contains(".understandable/intermediate/"));
        assert!(block.contains(".understandable/tmp/"));
        assert!(!block.contains("\n.understandable/\n"));
    }

    #[test]
    fn render_no_commit_ignores_whole_dir() {
        let s = settings(false, ".understandable");
        let block = render_block(GitignorePolicy::from_settings(&s), &s.storage);
        assert!(block.contains(".understandable/"));
        assert!(!block.contains("intermediate/"));
    }

    #[test]
    fn create_then_update_keeps_user_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, "node_modules/\n.idea/\n").unwrap();

        let s = settings(true, ".understandable");
        let outcome = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
        assert_eq!(outcome, GitignoreOutcome::Appended);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("node_modules/"));
        assert!(body.contains(".idea/"));
        assert!(body.contains(".understandable/intermediate/"));

        // Switch to ignore-all — block must be replaced in place.
        let s2 = settings(false, ".understandable");
        let outcome = apply_block(dir.path(), GitignorePolicy::IgnoreAll, &s2.storage).unwrap();
        assert_eq!(outcome, GitignoreOutcome::Updated);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains(".understandable/"));
        assert!(!body.contains("intermediate/"));
        assert!(body.contains("node_modules/"));
        assert!(body.contains(".idea/"));

        // Re-applying the same policy must be a no-op.
        let outcome = apply_block(dir.path(), GitignorePolicy::IgnoreAll, &s2.storage).unwrap();
        assert_eq!(outcome, GitignoreOutcome::AlreadyCurrent);
    }

    #[test]
    fn create_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let s = settings(true, ".cache/ua");
        let outcome = apply_block(dir.path(), GitignorePolicy::CommitDb, &s.storage).unwrap();
        assert_eq!(outcome, GitignoreOutcome::Created);
        let body = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(body.contains(".cache/ua/intermediate/"));
        assert!(body.contains(".cache/ua/tmp/"));
    }
}
