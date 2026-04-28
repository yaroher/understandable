//! Common embedding-provider trait.
//!
//! Lets the search command treat OpenAI / Ollama / local-ONNX behind
//! the same interface. Implementors live in `embeddings.rs` (HTTP) and
//! `local.rs` (fastembed-rs, behind the `local-embeddings` feature).

use async_trait::async_trait;
use ua_core::Error;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, Error>;
}

/// Deprecated alias kept for one release. The variants of the old
/// `ProviderError` (Generic, Embedding, Local) all collapse into
/// [`ua_core::Error`]'s `Embedding` / `LocalEmbedding` / `Provider`
/// string variants.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type ProviderError = Error;
