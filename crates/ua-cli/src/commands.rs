//! Subcommand wiring. Each module exposes `Args` (clap derive) and an
//! async `run(args, &project_path)` (or `run(args)` for project-less
//! commands).

pub mod analyze;
pub mod chat;
pub mod dashboard;
pub mod diff;
pub mod domain;
pub mod embed;
pub mod explain;
pub mod export;
pub mod extract;
pub mod fingerprint;
pub mod import;
pub mod init;
pub mod knowledge;
pub mod merge;
pub mod onboard;
pub mod scan;
pub mod search;
pub mod staleness;
pub mod usage;
pub mod validate;
