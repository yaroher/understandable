//! Markdown context builders consumed by the `chat` / `diff` / `explain`
//! / `onboard` subcommands. The functions here mirror the original
//! `src/diff-analyzer.ts`, `src/explain-builder.ts`,
//! `src/onboard-builder.ts` — all pure graph traversal + formatting,
//! no IO.

pub mod diff_analyzer;
pub mod explain_builder;
pub mod onboard_builder;
