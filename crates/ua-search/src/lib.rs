//! Fuzzy search over [`GraphNode`]s + chat-context builder.
//!
//! Replacement for `packages/core/src/search.ts` (Fuse.js) +
//! `src/context-builder.ts` from the original plugin. Ranking uses
//! `nucleo-matcher` with a weighted-field model that mirrors the Fuse
//! configuration: `name(0.4) + tags(0.3) + summary(0.2) +
//! language_notes(0.1)`.

pub mod context;
pub mod engine;

pub use context::{build_chat_context, format_context_for_prompt, ChatContext};
pub use engine::{SearchEngine, SearchOptions, SearchResult};

#[allow(unused_imports)]
use ua_core::GraphNode;
