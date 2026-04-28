//! Default model identifiers shared across the LLM and embedding
//! providers. Centralised so the CLI, the analyze pipeline and the
//! HTTP / local clients all agree on the same default strings — no
//! more drift between `commands/init.rs::apply_preset` and
//! `commands/embed.rs::default_model_for`.

/// Anthropic chat model used by `analyze --with-llm` and the chat
/// subcommand by default.
pub const ANTHROPIC_DEFAULT: &str = "claude-opus-4-7";

/// OpenAI embedding model used when the embeddings provider is
/// `openai` and no explicit model is configured.
pub const OPENAI_EMBED_DEFAULT: &str = "text-embedding-3-small";

/// Ollama embedding model used by the `ollama` shortcut in
/// `OpenAiEmbeddings::ollama` and the `ollama` provider preset.
pub const OLLAMA_EMBED_DEFAULT: &str = "nomic-embed-text";

/// Default fastembed-rs ONNX model used when `--embed-provider local`
/// is selected without an explicit `--embed-model` override.
pub const LOCAL_EMBED_DEFAULT: &str = "bge-small-en-v1.5";
