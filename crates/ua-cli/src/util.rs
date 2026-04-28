//! CLI-wide utility helpers shared across subcommands. Anything in
//! this module must stay free of business logic — it only exists to
//! deduplicate snippets that crept into multiple `commands/*.rs`
//! files (timestamps, path helpers, etc).

pub mod time;
