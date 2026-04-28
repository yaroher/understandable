//! Minimal Anthropic Messages client.
//!
//! Targets the public `/v1/messages` endpoint. The default model is
//! [`crate::models::ANTHROPIC_DEFAULT`] — pass an override via
//! [`CompleteRequest::model`] if you want something cheaper. The
//! client is `tokio + reqwest` based; cancel-safety is the caller's
//! responsibility.
//!
//! ## Prompt caching
//!
//! Set [`CompleteRequest::cache_system`] (or call
//! [`CompleteRequest::with_system_cache`]) to mark the system prompt as
//! cacheable. The client then sends the system field as an array
//! containing one `text` block with `cache_control: { type:
//! "ephemeral" }` and emits the `anthropic-beta:
//! prompt-caching-2024-07-31` header. Anthropic charges 1.25× on the
//! cache write and 0.1× on subsequent reads, so caching is worth it
//! whenever the same system prompt is reused across many requests.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use ua_core::Error;

use crate::models::ANTHROPIC_DEFAULT;
use crate::retry::{with_retry, RetryPolicy};

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const DEFAULT_VERSION: &str = "2023-06-01";
/// Beta header required by older API versions to enable prompt caching.
/// We always emit it when caching is requested, even on `2023-06-01`,
/// since the flag is harmless for newer revisions.
pub(crate) const PROMPT_CACHING_BETA: &str = "prompt-caching-2024-07-31";

/// Deprecated alias kept for one release. The variants are gone — they
/// fold into `ua_core::Error::Anthropic(String)` (and a couple of other
/// IO / serde buckets). New code should depend on
/// [`ua_core::Error`] directly.
#[deprecated(since = "0.2.0", note = "use `ua_core::Error` instead")]
pub type AnthropicError = Error;

#[derive(Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    /// Wrapped in `SecretString` so the key is redacted from `Debug`
    /// output, panic messages, and any structured logger that prints
    /// the struct verbatim.
    api_key: SecretString,
    base_url: String,
    api_version: String,
    /// Policy applied to the HTTP send/read step of `complete`.
    /// JSON parsing of a 2xx body is *not* retried — only transport
    /// failures and 429/5xx responses replay.
    retry: RetryPolicy,
}

impl std::fmt::Debug for AnthropicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicClient")
            .field("http", &"<reqwest::Client>")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("api_version", &self.api_version)
            .finish()
    }
}

impl AnthropicClient {
    /// Build a client. `api_key` defaults to `ANTHROPIC_API_KEY`.
    pub fn new(api_key: Option<String>) -> Result<Self, Error> {
        let api_key = api_key
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or_else(|| Error::Anthropic("missing ANTHROPIC_API_KEY".into()))?;
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("understandable/0.1")
                .build()
                .map_err(|e| Error::Anthropic(format!("http: {e}")))?,
            api_key: SecretString::from(api_key),
            base_url: DEFAULT_BASE.to_string(),
            api_version: DEFAULT_VERSION.to_string(),
            retry: RetryPolicy::default(),
        })
    }

    /// Override the default base URL — useful for routing through a
    /// proxy or hitting a self-hosted endpoint that speaks the same wire
    /// format.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the retry policy applied to `complete`. Pass
    /// [`RetryPolicy::no_retry`] to disable retries entirely.
    pub fn with_retry_policy(mut self, p: RetryPolicy) -> Self {
        self.retry = p;
        self
    }

    /// Borrow the configured retry policy. Used by [`crate::batch`] to
    /// share the same backoff config without re-plumbing builders.
    pub(crate) fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Borrow the API key. `pub(crate)` so [`crate::batch::BatchClient`]
    /// can construct itself from an existing client without leaking the
    /// secret to user code.
    pub(crate) fn api_key(&self) -> &SecretString {
        &self.api_key
    }

    /// Borrow the base URL.
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Borrow the API version.
    pub(crate) fn api_version(&self) -> &str {
        &self.api_version
    }

    /// Send one request, return the assembled text from every text-typed
    /// content block in the response.
    ///
    /// This is a thin wrapper around [`Self::complete_with_usage`] that
    /// drops the token-usage payload. Existing callers expect a
    /// `Result<String, Error>` — keep it that way.
    pub async fn complete(&self, req: CompleteRequest) -> Result<String, Error> {
        Ok(self.complete_with_usage(req).await?.text)
    }

    /// Send one request, return the assembled text *and* the token-usage
    /// numbers reported by the API (input, output, cache creation, cache
    /// read). Use this when you need to track caching efficiency or
    /// settle a per-call cost budget.
    pub async fn complete_with_usage(&self, req: CompleteRequest) -> Result<CompleteResult, Error> {
        let cache_system = req.cache_system;
        let body = build_api_request(&req);
        let url = format!("{}/v1/messages", self.base_url);
        // Only the HTTP roundtrip is retried. A 2xx response with a
        // malformed JSON body is a deterministic failure — no point
        // hammering the API for it.
        let text = with_retry(&self.retry, || async {
            let mut builder = self
                .http
                .post(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", &self.api_version)
                .header("content-type", "application/json");
            if cache_system {
                builder = builder.header("anthropic-beta", PROMPT_CACHING_BETA);
            }
            let res = builder
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Anthropic(format!("http: {e}")))?;
            let status = res.status();
            let text = res
                .text()
                .await
                .map_err(|e| Error::Anthropic(format!("body read: {e}")))?;
            if !status.is_success() {
                return Err(Error::Anthropic(format!(
                    "anthropic api: status={} body={text}",
                    status.as_u16()
                )));
            }
            Ok(text)
        })
        .await?;
        let parsed: ApiResponse = serde_json::from_str(&text)?;
        let mut out = String::new();
        for block in parsed.content {
            if block.kind == "text" {
                if let Some(t) = block.text {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&t);
                }
            }
        }
        if out.is_empty() {
            return Err(Error::Anthropic("response had no text content".into()));
        }
        Ok(CompleteResult {
            text: out,
            usage: parsed.usage.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// One Anthropic completion request.
///
/// All fields are optional except `messages`. `cache_system` toggles
/// the prompt-cache wire format described at the top of this module.
#[derive(Debug, Clone, Default)]
pub struct CompleteRequest {
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// When `true`, the system prompt is sent as a cacheable content
    /// block. Has no effect when `system` is `None`.
    pub cache_system: bool,
}

impl CompleteRequest {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            messages: vec![ChatMessage::user(content)],
            ..Default::default()
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_max_tokens(mut self, max: u32) -> Self {
        self.max_tokens = Some(max);
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }

    /// Mark the system prompt as cacheable. No-op if `system` is unset.
    pub fn with_system_cache(mut self) -> Self {
        self.cache_system = true;
        self
    }

    /// Explicit setter for the cache flag. Useful when wiring the value
    /// from a config struct rather than chaining a builder.
    pub fn with_cache_system(mut self, b: bool) -> Self {
        self.cache_system = b;
        self
    }
}

/// Translate a [`CompleteRequest`] into the wire-level [`ApiRequest`]
/// payload. Pulled out of [`AnthropicClient::complete`] so the Batch
/// API client can build identical request bodies without duplicating
/// the field plumbing.
pub(crate) fn build_api_request(req: &CompleteRequest) -> ApiRequest {
    // Clone the system string at most once. The previous version had
    // a `s.clone()` call in two separate match arms; consolidating
    // here means a single textual clone-site that adapts based on the
    // cache flag.
    let system = req.system.as_ref().map(|s| {
        let owned = s.clone();
        if req.cache_system {
            SystemField::Blocks(vec![SystemBlock {
                kind: "text",
                text: owned,
                cache_control: Some(CacheControl { kind: "ephemeral" }),
            }])
        } else {
            SystemField::Text(owned)
        }
    });
    ApiRequest {
        model: req
            .model
            .clone()
            .unwrap_or_else(|| ANTHROPIC_DEFAULT.to_string()),
        max_tokens: req.max_tokens.unwrap_or(4096),
        system,
        messages: req.messages.clone(),
        temperature: req.temperature,
    }
}

/// Wire-level shape of a `/v1/messages` request body. Exposed
/// `pub(crate)` so the Batch API can serialise it under a `params`
/// key.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ApiRequest {
    pub(crate) model: String,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<SystemField>,
    pub(crate) messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
}

/// The `system` field accepts either a plain string or an array of
/// content blocks. We use the block form only when caching is on.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum SystemField {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

/// One entry in the `system` array. Mirrors Anthropic's content-block
/// shape; only `text` blocks are emitted by this client.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SystemBlock {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<CacheControl>,
}

/// The single supported cache-control marker. `type: "ephemeral"`
/// gives a ~5-minute TTL.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CacheControl {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
}

/// The text + token-usage tuple returned by
/// [`AnthropicClient::complete_with_usage`].
#[derive(Debug, Clone)]
pub struct CompleteResult {
    /// All `text` blocks of the response, joined with newlines.
    pub text: String,
    /// Token-usage figures parsed from the response body. Defaults to
    /// zero when the API doesn't include a `usage` field (older
    /// versions, or non-success responses we somehow accept).
    pub usage: TokenUsage,
}

/// Token accounting reported by the Anthropic API. The two cache
/// fields are populated when prompt caching is in use:
///
/// * `cache_creation_input_tokens` — tokens written to a fresh cache
///   block; billed at 1.25× the input rate.
/// * `cache_read_input_tokens` — tokens served from the cache; billed
///   at 0.1× the input rate.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_api_request_serialises_with_cache_block_when_cache_system_set() {
        let req = CompleteRequest::user("hi")
            .with_system("you are helpful")
            .with_system_cache();
        let body = build_api_request(&req);
        let v = serde_json::to_value(&body).expect("serialise");
        // Block form with cache_control.
        assert_eq!(
            v["system"],
            json!([
                {
                    "type": "text",
                    "text": "you are helpful",
                    "cache_control": { "type": "ephemeral" },
                }
            ])
        );
        // Sanity: messages still ride along untouched.
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn build_api_request_serialises_plain_string_when_cache_off() {
        let req = CompleteRequest::user("hi").with_system("you are helpful");
        let body = build_api_request(&req);
        let v = serde_json::to_value(&body).expect("serialise");
        assert_eq!(v["system"], json!("you are helpful"));
    }

    #[test]
    fn build_api_request_omits_system_when_none() {
        let req = CompleteRequest::user("hi");
        let body = build_api_request(&req);
        let v = serde_json::to_value(&body).expect("serialise");
        assert!(v.get("system").is_none(), "system should be omitted");
    }

    #[test]
    fn complete_result_parses_usage() {
        // Hand-rolled fixture matching the Anthropic Messages response
        // shape — checks both `content` text extraction and `usage`
        // deserialisation in one shot.
        let body = json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "hello world" }
            ],
            "model": "claude-x",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "cache_creation_input_tokens": 56,
                "cache_read_input_tokens": 78
            }
        });
        let parsed: ApiResponse = serde_json::from_value(body).expect("parse");
        let usage = parsed.usage.expect("usage present");
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.output_tokens, 34);
        assert_eq!(usage.cache_creation_input_tokens, 56);
        assert_eq!(usage.cache_read_input_tokens, 78);
        assert_eq!(parsed.content.len(), 1);
        assert_eq!(parsed.content[0].kind, "text");
        assert_eq!(parsed.content[0].text.as_deref(), Some("hello world"));
    }

    #[test]
    fn complete_result_usage_defaults_when_missing() {
        // Older API revisions / partial mocks may omit the field.
        let body = json!({
            "content": [ { "type": "text", "text": "ok" } ]
        });
        let parsed: ApiResponse = serde_json::from_value(body).expect("parse");
        assert!(parsed.usage.is_none());
    }
}
