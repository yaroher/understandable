//! Staleness detection — port of `staleness.ts`.
//!
//! Compares the persisted graph's `git_commit_hash` (in `meta`) against
//! the current `HEAD` and reports a [`StalenessStatus`].

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StalenessStatus {
    Fresh,
    Stale { persisted: String, current: String },
    NoGraph,
    NoGit,
}

#[derive(Debug, Clone)]
pub struct StalenessReport {
    pub status: StalenessStatus,
}

pub fn current_git_head(project_root: impl AsRef<Path>) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(project_root.as_ref())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn report(persisted_hash: Option<&str>, current_hash: Option<&str>) -> StalenessReport {
    let status = match (persisted_hash, current_hash) {
        (None, _) => StalenessStatus::NoGraph,
        (_, None) => StalenessStatus::NoGit,
        (Some(p), Some(c)) if p == c => StalenessStatus::Fresh,
        (Some(p), Some(c)) => StalenessStatus::Stale {
            persisted: p.to_string(),
            current: c.to_string(),
        },
    };
    StalenessReport { status }
}
