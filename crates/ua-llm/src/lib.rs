//! HTTP + local-ONNX clients for the standalone enrichment pipeline
//! and the embedding-search backend.
//!
//! Three embedding providers are available:
//!   * [`OpenAiEmbeddings`] — talks to OpenAI or any OpenAI-compatible
//!     endpoint. With `OpenAiEmbeddings::ollama` it points at Ollama
//!     out of the box (no auth header).
//!   * [`local::LocalEmbeddings`] — fastembed-rs running ONNX models
//!     on the local CPU/GPU. Behind the `local-embeddings` feature
//!     flag because it brings native deps + downloads model weights.
//!
//! Both implement [`provider::EmbeddingProvider`] so callers can swap
//! without thinking about the underlying transport.

pub mod anthropic;
pub mod batch;
pub mod embeddings;
#[cfg(feature = "local-embeddings")]
pub mod local;
pub mod models;
pub mod prompts;
pub mod provider;
pub mod retry;

#[allow(deprecated)]
pub use anthropic::{
    AnthropicClient, AnthropicError, ChatMessage, ChatRole, CompleteRequest, CompleteResult,
    TokenUsage,
};
pub use batch::{
    BatchClient, BatchCounts, BatchRequest, BatchResultBody, BatchResultLine, BatchSubmitResponse,
};
#[allow(deprecated)]
pub use embeddings::{cosine_similarity, EmbeddingError, OpenAiEmbeddings};
#[cfg(feature = "local-embeddings")]
#[allow(deprecated)]
pub use local::{EmbeddingModel as LocalEmbeddingModel, LocalEmbeddingError, LocalEmbeddings};
pub use models::{
    ANTHROPIC_DEFAULT, LOCAL_EMBED_DEFAULT, OLLAMA_EMBED_DEFAULT, OPENAI_EMBED_DEFAULT,
};
pub use prompts::{
    file_summary_prompts, parse_file_summary, FileSummaryRequest, FileSummaryResponse,
};
#[allow(deprecated)]
pub use provider::{EmbeddingProvider, ProviderError};
pub use retry::{is_retryable, with_retry, RetryPolicy};

/// Re-export the workspace-wide error so callers don't have to depend
/// on `ua_core` directly.
pub use ua_core::Error;

/// Deprecated alias kept for one release. The aggregate error is
/// gone — callers should propagate [`ua_core::Error`] directly. The
/// alias exists only so existing import lines keep compiling.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type LlmError = ua_core::Error;
