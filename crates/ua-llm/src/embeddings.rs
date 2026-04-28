//! OpenAI-compatible embeddings client + cosine helper.
//!
//! Used by `ua-search` to lift the LIKE-prefiltered candidate set into
//! a semantic-similarity ranker. Defaults to
//! [`crate::models::OPENAI_EMBED_DEFAULT`] — pass `--model` if you
//! want a different one. Works against any OpenAI-compatible endpoint
//! (Azure OpenAI, local llama.cpp servers, etc.) by overriding the
//! base URL.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use ua_core::Error;

use crate::models::OPENAI_EMBED_DEFAULT;
use crate::provider::EmbeddingProvider;
use crate::retry::{with_retry, RetryPolicy};

const DEFAULT_BASE: &str = "https://api.openai.com";

/// Deprecated alias kept for one release. Variants fold into
/// `ua_core::Error::Embedding(String)`.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type EmbeddingError = Error;

#[derive(Clone)]
pub struct OpenAiEmbeddings {
    http: reqwest::Client,
    /// Wrapped in `SecretString` so the key is redacted from `Debug`
    /// output. Keyless mode (Ollama, llama.cpp) uses an empty secret;
    /// the `Authorization` header is then omitted entirely below.
    api_key: SecretString,
    base_url: String,
    model: String,
    /// Policy applied to the HTTP send/read step of `embed`. JSON
    /// parsing of a 2xx body is *not* retried.
    retry: RetryPolicy,
}

impl std::fmt::Debug for OpenAiEmbeddings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbeddings")
            .field("http", &"<reqwest::Client>")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish()
    }
}

impl OpenAiEmbeddings {
    pub fn new(api_key: Option<String>) -> Result<Self, Error> {
        let api_key = api_key
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| Error::Embedding("missing OPENAI_API_KEY".into()))?;
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("understandable/0.1")
                .build()
                .map_err(|e| Error::Embedding(format!("http: {e}")))?,
            api_key: SecretString::from(api_key),
            base_url: DEFAULT_BASE.to_string(),
            model: OPENAI_EMBED_DEFAULT.to_string(),
            retry: RetryPolicy::default(),
        })
    }

    /// Build a client without requiring an API key — for local
    /// OpenAI-compatible servers (Ollama, llama.cpp, vllm, …) that
    /// ignore the `Authorization` header.
    pub fn new_keyless() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("understandable/0.1")
                .build()
                .expect("reqwest client"),
            api_key: SecretString::from(String::new()),
            base_url: DEFAULT_BASE.to_string(),
            model: OPENAI_EMBED_DEFAULT.to_string(),
            retry: RetryPolicy::default(),
        }
    }

    /// Convenience for the most common local-server case:
    /// `OpenAiEmbeddings::ollama(ua_llm::OLLAMA_EMBED_DEFAULT)` →
    /// talks to `http://127.0.0.1:11434` with no auth header. Pass any
    /// other model name to point Ollama at a different local model.
    pub fn ollama(model: impl Into<String>) -> Self {
        Self::new_keyless()
            .with_base_url("http://127.0.0.1:11434")
            .with_model(model)
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the retry policy applied to `embed`. Pass
    /// [`RetryPolicy::no_retry`] to disable retries entirely.
    pub fn with_retry_policy(mut self, p: RetryPolicy) -> Self {
        self.retry = p;
        self
    }

    /// Embed every input string in one batched call. Returns one vector
    /// per input, in order.
    pub async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        let body = ApiRequest {
            model: self.model.clone(),
            input: inputs.iter().map(|s| s.to_string()).collect(),
        };
        let url = format!("{}/v1/embeddings", self.base_url);
        // Only the HTTP roundtrip is retried; a 2xx with malformed
        // JSON is a deterministic decode failure.
        let text = with_retry(&self.retry, || async {
            let mut req = self
                .http
                .post(&url)
                .header("content-type", "application/json")
                .json(&body);
            // When `api_key` is empty (i.e. `new_keyless()` / `ollama()`),
            // we deliberately omit the `Authorization` header entirely.
            // Sending `Authorization: Bearer ` (empty bearer) makes strict
            // local stacks (LiteLLM, vllm with auth-required) reject the
            // request rather than silently ignore it. Only attach the
            // header when the caller actually supplied a key.
            let key = self.api_key.expose_secret();
            if !key.is_empty() {
                req = req.bearer_auth(key);
            }
            let res = req
                .send()
                .await
                .map_err(|e| Error::Embedding(format!("http: {e}")))?;
            let status = res.status();
            let text = res
                .text()
                .await
                .map_err(|e| Error::Embedding(format!("body read: {e}")))?;
            if !status.is_success() {
                return Err(Error::Embedding(format!(
                    "openai api: status={} body={text}",
                    status.as_u16()
                )));
            }
            Ok(text)
        })
        .await?;
        let parsed: ApiResponse = serde_json::from_str(&text)?;
        if parsed.data.is_empty() {
            return Err(Error::Embedding("response missing embedding data".into()));
        }
        let mut sorted = parsed.data;
        sorted.sort_by_key(|d| d.index);
        Ok(sorted.into_iter().map(|d| d.embedding).collect())
    }
}

/// Cosine similarity in `[-1, 1]`. Zero-magnitude vectors return `0.0`.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddings {
    async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        self.embed(inputs).await
    }
}

#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_orthogonal() {
        let a = [1.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_identical() {
        let a = [1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_safe() {
        let a = [0.0, 0.0];
        let b = [1.0, 1.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }
}
