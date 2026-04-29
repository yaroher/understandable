//! `understandable staleness` — compare the persisted graph's
//! `git_commit_hash` against the current `HEAD` and report drift.
//!
//! Exit codes (stable contract — the post-commit / SessionStart hook in
//! `plugin/hooks/hooks.json` branches on these):
//!
//! | Code | Meaning                                                     |
//! |------|-------------------------------------------------------------|
//! |  0   | Fresh — graph commit == current HEAD.                       |
//! |  1   | Stale — graph commit differs from HEAD (drift detected).    |
//! |  2   | No graph — `<storage>/<db>.tar.zst` does not exist yet.     |
//! |  3   | Error — git not available or some other failure.            |
//!
//! `--json` switches stdout to a single JSON document with shape:
//!
//! ```json
//! {
//!   "stale": true,
//!   "current_commit": "<sha>",
//!   "graph_commit": "<sha>",
//!   "drift_count": 7
//! }
//! ```
//!
//! `drift_count` is the number of files reported by
//! `git diff --name-only <graph_commit>..HEAD`. When fresh,
//! `drift_count == 0`. When the graph has no commit hash recorded,
//! `drift_count` is `null`.

use std::path::Path;

use clap::Args as ClapArgs;
use serde::Serialize;
use ua_persist::staleness::{current_git_head, report as staleness_report, StalenessStatus};
use ua_persist::{ProjectLayout, Storage};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Emit a single JSON document on stdout instead of human text.
    #[arg(long)]
    pub json: bool,
    /// Suppress all stdout — only the exit code is significant. Useful
    /// for shell hooks that want to branch on `&&` / `||` without any
    /// noise.
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Debug, Serialize)]
struct StalenessJson {
    stale: bool,
    current_commit: Option<String>,
    graph_commit: Option<String>,
    drift_count: Option<usize>,
}

pub async fn run(args: Args, project_path: &Path) -> anyhow::Result<()> {
    let layout = ProjectLayout::for_project(project_path);
    let archive_path = layout.graph_archive();

    // Exit 2 — no graph at all. Don't open Storage (which would happily
    // return an empty in-memory store for a non-existent archive).
    if !archive_path.exists() {
        emit(
            args.json,
            args.quiet,
            &StalenessJson {
                stale: false,
                current_commit: current_git_head(project_path),
                graph_commit: None,
                drift_count: None,
            },
            "no graph",
        );
        std::process::exit(2);
    }

    let storage = Storage::open(&layout).await?;
    let graph = storage.load_graph().await?;
    let persisted = graph.project.git_commit_hash.trim().to_string();
    let persisted_opt = if persisted.is_empty() {
        None
    } else {
        Some(persisted.as_str())
    };

    let current = current_git_head(project_path);
    let report = staleness_report(persisted_opt, current.as_deref());

    match report.status {
        StalenessStatus::Fresh => {
            emit(
                args.json,
                args.quiet,
                &StalenessJson {
                    stale: false,
                    current_commit: current.clone(),
                    graph_commit: persisted_opt.map(|s| s.to_string()),
                    drift_count: Some(0),
                },
                "fresh",
            );
            std::process::exit(0);
        }
        StalenessStatus::Stale {
            persisted,
            current: cur,
        } => {
            let drift = git_diff_count(project_path, &persisted, &cur).ok();
            emit(
                args.json,
                args.quiet,
                &StalenessJson {
                    stale: true,
                    current_commit: Some(cur.clone()),
                    graph_commit: Some(persisted.clone()),
                    drift_count: drift,
                },
                "stale",
            );
            std::process::exit(1);
        }
        StalenessStatus::NoGraph => {
            // Archive exists but `meta.project.git_commit_hash` is empty.
            // Treat as stale — the graph was built without a recorded
            // commit, so we can't prove freshness. Drift count is
            // unknowable.
            emit(
                args.json,
                args.quiet,
                &StalenessJson {
                    stale: true,
                    current_commit: current.clone(),
                    graph_commit: None,
                    drift_count: None,
                },
                "stale (graph has no commit hash recorded)",
            );
            std::process::exit(1);
        }
        StalenessStatus::NoGit => {
            emit(
                args.json,
                args.quiet,
                &StalenessJson {
                    stale: false,
                    current_commit: None,
                    graph_commit: persisted_opt.map(|s| s.to_string()),
                    drift_count: None,
                },
                "no git HEAD available",
            );
            std::process::exit(3);
        }
    }
}

fn emit(json: bool, quiet: bool, payload: &StalenessJson, human: &str) {
    if quiet {
        return;
    }
    if json {
        // Single deterministic line — pretty-printed for skim-ability.
        match serde_json::to_string_pretty(payload) {
            Ok(s) => println!("{s}"),
            Err(_) => println!("{{\"stale\": {}}}", payload.stale),
        }
    } else {
        let cur = payload.current_commit.as_deref().unwrap_or("-");
        let g = payload.graph_commit.as_deref().unwrap_or("-");
        match payload.drift_count {
            Some(n) => println!("{human} (graph={g} HEAD={cur} drift={n})"),
            None => println!("{human} (graph={g} HEAD={cur})"),
        }
    }
}

/// Count the files reported by `git diff --name-only <a>..<b>`. A
/// missing commit on either side (e.g. shallow clone, rewritten history)
/// is surfaced as an error so the caller can render `null`.
fn git_diff_count(project_root: &Path, a: &str, b: &str) -> Result<usize, String> {
    let out = std::process::Command::new("git")
        .arg("diff")
        .arg("--name-only")
        .arg(format!("{a}..{b}"))
        .current_dir(project_root)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "git diff exited with status {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines().filter(|l| !l.trim().is_empty()).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: the JSON payload serialises with the documented
    /// top-level keys. Hooks parse this contract; if a key is renamed
    /// the test catches it.
    #[test]
    fn json_payload_shape() {
        let p = StalenessJson {
            stale: true,
            current_commit: Some("aaa".into()),
            graph_commit: Some("bbb".into()),
            drift_count: Some(3),
        };
        let s = serde_json::to_string(&p).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("parse");
        assert_eq!(v.get("stale").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(
            v.get("current_commit").and_then(|x| x.as_str()),
            Some("aaa")
        );
        assert_eq!(v.get("graph_commit").and_then(|x| x.as_str()), Some("bbb"));
        assert_eq!(v.get("drift_count").and_then(|x| x.as_u64()), Some(3));
    }

    #[test]
    fn json_payload_nullable_fields() {
        let p = StalenessJson {
            stale: false,
            current_commit: None,
            graph_commit: None,
            drift_count: None,
        };
        let s = serde_json::to_string(&p).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&s).expect("parse");
        assert!(v.get("current_commit").unwrap().is_null());
        assert!(v.get("graph_commit").unwrap().is_null());
        assert!(v.get("drift_count").unwrap().is_null());
    }
}
