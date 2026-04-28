//! Local ONNX embeddings via [fastembed-rs][fastembed].
//!
//! Behind the `local-embeddings` feature flag because it pulls in
//! `ort` / `tokenizers` / `hf-hub` (~30-100 MB native deps) and
//! downloads model weights on first run. Default model is
//! `BAAI/bge-small-en-v1.5` — small (~120 MB), fast on CPU, decent
//! quality. Override via [`LocalEmbeddings::with_model`].
//!
//! [fastembed]: https://github.com/Anush008/fastembed-rs

use async_trait::async_trait;
pub use fastembed::EmbeddingModel;
use fastembed::{InitOptions, TextEmbedding, TextInitOptions};
use ua_core::Error;

use crate::provider::EmbeddingProvider;

/// Deprecated alias kept for one release. Variants fold into
/// `ua_core::Error::LocalEmbedding(String)`.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type LocalEmbeddingError = Error;

pub struct LocalEmbeddings {
    // `TextEmbedding::embed` takes `&mut self`. Wrapping in a Mutex lets
    // us share one model across requests without re-loading the ONNX
    // session per call.
    inner: std::sync::Arc<std::sync::Mutex<TextEmbedding>>,
    batch_size: usize,
}

impl LocalEmbeddings {
    /// Initialise with the default `BAAI/bge-small-en-v1.5` model.
    pub fn new() -> Result<Self, Error> {
        Self::with_options(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(false),
        )
    }

    /// Build with a custom [`EmbeddingModel`].
    pub fn with_model(model: EmbeddingModel) -> Result<Self, Error> {
        Self::with_options(InitOptions::new(model).with_show_download_progress(false))
    }

    pub fn with_options(opts: InitOptions) -> Result<Self, Error> {
        let inner = TextEmbedding::try_new(TextInitOptions::from(opts))
            .map_err(|e| Error::LocalEmbedding(format!("fastembed init: {e}")))?;
        Ok(Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(inner)),
            batch_size: 16,
        })
    }

    pub fn with_batch_size(mut self, n: usize) -> Self {
        self.batch_size = n.max(1);
        self
    }

    pub async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        let owned: Vec<String> = inputs.iter().map(|s| (*s).to_string()).collect();
        let batch_size = Some(self.batch_size);
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            // Recover from poisoning — fastembed has no internal state
            // mutated mid-call we can't retry over, so a panicked prior
            // batch shouldn't permanently brick the whole client.
            let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .embed(owned, batch_size)
                .map_err(|e| Error::LocalEmbedding(format!("fastembed embed: {e}")))
        })
        .await
        .map_err(|e| Error::LocalEmbedding(format!("blocking task failed: {e}")))?
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddings {
    async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, Error> {
        self.embed(inputs).await
    }
}
