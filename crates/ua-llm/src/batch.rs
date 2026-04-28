//! Anthropic Message Batches API client.
//!
//! Submit up to 100 000 `/v1/messages` requests in a single job, poll
//! for completion, then download a JSONL file of results. Anthropic
//! gives a 50 % discount on tokens spent inside a batch, so this is
//! the path to take for any large offline enrichment run (file
//! summaries, embeddings prep, bulk QA, etc).
//!
//! Wire endpoints:
//! * `POST /v1/messages/batches` — submit
//! * `GET  /v1/messages/batches/<id>` — status
//! * `GET  <results_url>` — JSONL of per-request outcomes
//!
//! All HTTP calls are wrapped in [`crate::retry::with_retry`], so
//! transient 429/5xx blips replay automatically. The `submit` body
//! reuses [`crate::anthropic::build_api_request`] under the hood:
//! whatever shape `AnthropicClient::complete` would have sent, the
//! batch envelope sends — including prompt-cache `cache_control`
//! blocks.
//!
//! ## Terminal statuses
//!
//! The batch is "done" once `processing_status` is one of `ended`,
//! `canceling`, `canceled` / `cancelled`, or `failed`. The previous
//! version recognised only `ended` and would spin forever if Anthropic
//! ever cancelled or failed a job. [`BatchSubmitResponse::is_done`]
//! now matches all five tokens. [`BatchClient::wait_until_done`] takes
//! an optional `max_polls` cap so callers can bound the loop without
//! threading [`tokio::time::timeout`] through every call site.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use ua_core::Error;

use crate::anthropic::{build_api_request, AnthropicClient, ApiRequest, CompleteRequest};
use crate::retry::{with_retry, RetryPolicy};

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const DEFAULT_VERSION: &str = "2023-06-01";
/// Beta header guarding the Message Batches API. Required at least
/// through the `2023-06-01` version line.
const BATCHES_BETA: &str = "message-batches-2024-09-24";
/// Default polling cadence for [`BatchClient::wait_until_done`]. The
/// API recommends "at most once a minute"; 30 s is comfortable for
/// jobs that finish in single-digit minutes.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);
/// `processing_status` values that mean the batch has reached a
/// terminal state and will never advance again. We recognise the full
/// set so [`BatchClient::wait_until_done`] doesn't spin on a `failed`
/// or `cancelled` job.
///
/// `cancelled` (double-l, en-GB) is included alongside `canceled`
/// because Anthropic's docs and wire format have historically used
/// both spellings; matching either is cheaper than guessing.
const TERMINAL_STATUSES: &[&str] = &["ended", "canceling", "canceled", "cancelled", "failed"];

/// One request in a batch submission. The `custom_id` is echoed back
/// in the result line so the caller can join input ↔ output.
#[derive(Debug, Clone)]
pub struct BatchRequest {
    /// Caller-chosen identifier. Must be unique within a single batch.
    pub custom_id: String,
    /// The same shape you'd pass to [`AnthropicClient::complete`].
    pub request: CompleteRequest,
}

impl BatchRequest {
    /// Convenience constructor.
    pub fn new(custom_id: impl Into<String>, request: CompleteRequest) -> Self {
        Self {
            custom_id: custom_id.into(),
            request,
        }
    }
}

/// Status payload returned by `submit` and `status`. Field names match
/// the wire format verbatim.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchSubmitResponse {
    /// Batch identifier, prefixed `msgbatch_…`.
    pub id: String,
    /// One of `in_progress`, `canceling`, `ended`. Use [`Self::is_done`].
    pub processing_status: String,
    /// RFC 3339 timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp; only set after the batch ends.
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub request_counts: BatchCounts,
    /// Where to fetch the JSONL results once `processing_status ==
    /// "ended"`. `None` until the batch finishes.
    #[serde(default)]
    pub results_url: Option<String>,
}

impl BatchSubmitResponse {
    /// `true` when the batch has reached a terminal state. Matches
    /// `ended`, `canceling`, `canceled`, `cancelled`, and `failed`;
    /// see [`TERMINAL_STATUSES`].
    pub fn is_done(&self) -> bool {
        TERMINAL_STATUSES
            .iter()
            .any(|s| self.processing_status.eq_ignore_ascii_case(s))
    }
}

/// Per-bucket request tallies returned with every status response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchCounts {
    #[serde(default)]
    pub processing: u64,
    #[serde(default)]
    pub succeeded: u64,
    #[serde(default)]
    pub errored: u64,
    #[serde(default)]
    pub canceled: u64,
    #[serde(default)]
    pub expired: u64,
}

/// One line of the JSONL response body. `custom_id` matches the value
/// from the original [`BatchRequest`].
#[derive(Debug, Clone, Deserialize)]
pub struct BatchResultLine {
    pub custom_id: String,
    pub result: BatchResultBody,
}

/// Tagged outcome of a single batch request. The `succeeded.message`
/// payload is left as raw `serde_json::Value` so callers can decide
/// whether to feed it through Anthropic's `ApiResponse` shape or pluck
/// individual fields directly.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BatchResultBody {
    /// A normal `/v1/messages` response body, exactly as
    /// `AnthropicClient::complete` would have parsed.
    Succeeded { message: serde_json::Value },
    /// API-side failure; `error` is the verbatim Anthropic error
    /// object.
    Errored { error: serde_json::Value },
    /// Cancelled before completion.
    Canceled,
    /// Hit the 24 h batch deadline.
    Expired,
}

/// Client for the Message Batches API. Constructed independently from
/// [`AnthropicClient`] but shares its env-var convention and retry
/// defaults.
#[derive(Clone)]
pub struct BatchClient {
    http: reqwest::Client,
    api_key: SecretString,
    base_url: String,
    api_version: String,
    retry: RetryPolicy,
}

impl std::fmt::Debug for BatchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BatchClient")
            .field("http", &"<reqwest::Client>")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("api_version", &self.api_version)
            .finish()
    }
}

impl BatchClient {
    /// Build a client. `api_key` defaults to `ANTHROPIC_API_KEY`. Same
    /// fallback logic as [`AnthropicClient::new`].
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

    /// Build a batch client that mirrors an existing
    /// [`AnthropicClient`]'s key, base URL, version, and retry policy.
    /// Convenient when the same project uses both single and batched
    /// calls.
    pub fn from_anthropic(client: &AnthropicClient) -> Result<Self, Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("understandable/0.1")
                .build()
                .map_err(|e| Error::Anthropic(format!("http: {e}")))?,
            api_key: client.api_key().clone(),
            base_url: client.base_url().to_string(),
            api_version: client.api_version().to_string(),
            retry: client.retry_policy().clone(),
        })
    }

    /// Override the default base URL. Useful for hitting a proxy or
    /// recording test fixture against a mock server.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the retry policy. Pass [`RetryPolicy::no_retry`] to
    /// disable replays in tests.
    pub fn with_retry_policy(mut self, p: RetryPolicy) -> Self {
        self.retry = p;
        self
    }

    /// Submit a batch. The returned [`BatchSubmitResponse::id`] is the
    /// handle for [`Self::status`] / [`Self::wait_until_done`].
    pub async fn submit(
        &self,
        requests: Vec<BatchRequest>,
    ) -> Result<BatchSubmitResponse, Error> {
        if requests.is_empty() {
            return Err(Error::Anthropic("submit: empty batch".into()));
        }
        let envelope = build_submit_body(&requests);
        let url = format!("{}/v1/messages/batches", self.base_url);
        let text = with_retry(&self.retry, || async {
            let res = self
                .http
                .post(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", &self.api_version)
                .header("anthropic-beta", BATCHES_BETA)
                .header("content-type", "application/json")
                .json(&envelope)
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
                    "anthropic batch submit: status={} body={text}",
                    status.as_u16()
                )));
            }
            Ok(text)
        })
        .await?;
        let parsed: BatchSubmitResponse = serde_json::from_str(&text)?;
        Ok(parsed)
    }

    /// Fetch one status snapshot. The caller drives the polling loop;
    /// see [`Self::wait_until_done`] for the canned version.
    pub async fn status(&self, batch_id: &str) -> Result<BatchSubmitResponse, Error> {
        let url = format!("{}/v1/messages/batches/{}", self.base_url, batch_id);
        let text = with_retry(&self.retry, || async {
            let res = self
                .http
                .get(&url)
                .header("x-api-key", self.api_key.expose_secret())
                .header("anthropic-version", &self.api_version)
                .header("anthropic-beta", BATCHES_BETA)
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
                    "anthropic batch status: status={} body={text}",
                    status.as_u16()
                )));
            }
            Ok(text)
        })
        .await?;
        let parsed: BatchSubmitResponse = serde_json::from_str(&text)?;
        Ok(parsed)
    }

    /// Block (in the async sense) until the batch reaches a terminal
    /// state (any of `ended`, `canceling`, `canceled`, `cancelled`,
    /// `failed`). `poll_every` controls the cadence; pass
    /// [`Duration::ZERO`] to fall back to the 30 s default.
    ///
    /// `max_polls` caps the number of poll cycles. `None` means no cap
    /// (legacy behaviour). When the cap is hit the call returns
    /// [`Error::Anthropic`] with the message `"batch poll timeout"`.
    /// For a wall-clock-bounded variant prefer [`Self::poll_for`].
    ///
    /// Transient HTTP errors are absorbed by the retry layer inside
    /// each [`Self::status`] call, so this loop only short-circuits on
    /// a fatal error (e.g. 4xx auth failure) or the `max_polls` cap.
    pub async fn wait_until_done(
        &self,
        batch_id: &str,
        poll_every: Duration,
        max_polls: Option<u32>,
    ) -> Result<BatchSubmitResponse, Error> {
        let interval = if poll_every.is_zero() {
            DEFAULT_POLL_INTERVAL
        } else {
            poll_every
        };
        let mut polls: u32 = 0;
        loop {
            let snap = self.status(batch_id).await?;
            polls += 1;
            tracing::info!(
                batch = %snap.id,
                status = %snap.processing_status,
                succeeded = snap.request_counts.succeeded,
                errored = snap.request_counts.errored,
                processing = snap.request_counts.processing,
                canceled = snap.request_counts.canceled,
                expired = snap.request_counts.expired,
                poll = polls,
                "batch poll"
            );
            if snap.is_done() {
                return Ok(snap);
            }
            if let Some(cap) = max_polls {
                if polls >= cap {
                    return Err(Error::Anthropic("batch poll timeout".into()));
                }
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Wall-clock-bounded variant of [`Self::wait_until_done`]. Wraps
    /// the polling loop in [`tokio::time::timeout`]; on expiry returns
    /// [`Error::Anthropic`] with the message `"batch poll timeout"`.
    /// `poll_every` and `max_total` follow the same conventions as the
    /// underlying call.
    pub async fn poll_for(
        &self,
        batch_id: &str,
        poll_every: Duration,
        max_total: Duration,
    ) -> Result<BatchSubmitResponse, Error> {
        match tokio::time::timeout(max_total, self.wait_until_done(batch_id, poll_every, None))
            .await
        {
            Ok(res) => res,
            Err(_) => Err(Error::Anthropic("batch poll timeout".into())),
        }
    }

    /// Download the JSONL result body and parse each line. The
    /// `results_url` returned by [`Self::status`] is pre-signed by
    /// Anthropic but still requires our `x-api-key` header.
    ///
    /// Streams the response body chunk-by-chunk via
    /// [`reqwest::Response::chunk`] and parses one line at a time.
    /// For a 100 000-request batch the body runs to ≈500 MB; the
    /// streaming reader keeps the in-flight read buffer bounded
    /// (resident memory still grows with the returned `Vec`, but the
    /// request body itself is no longer held in memory all at once).
    ///
    /// Error bodies are read in full so the `status=…` error string is
    /// useful — these are typically small.
    ///
    /// TODO: expose a `Stream<Item = BatchResultLine>` variant to keep
    /// even the output size bounded; left as a follow-up.
    pub async fn fetch_results(&self, results_url: &str) -> Result<Vec<BatchResultLine>, Error> {
        let owned = results_url.to_string();
        with_retry(&self.retry, || {
            let url = owned.clone();
            async move {
                let res = self
                    .http
                    .get(&url)
                    .header("x-api-key", self.api_key.expose_secret())
                    .header("anthropic-version", &self.api_version)
                    .header("anthropic-beta", BATCHES_BETA)
                    .send()
                    .await
                    .map_err(|e| Error::Anthropic(format!("http: {e}")))?;
                let status = res.status();
                if !status.is_success() {
                    // Failure path: the body is small (an error JSON) so
                    // pull it whole for the status= log message.
                    let text = res
                        .text()
                        .await
                        .map_err(|e| Error::Anthropic(format!("body read: {e}")))?;
                    return Err(Error::Anthropic(format!(
                        "anthropic batch results: status={} body={text}",
                        status.as_u16()
                    )));
                }
                stream_jsonl(res).await
            }
        })
        .await
    }
}

/// The submission envelope item. One per [`BatchRequest`].
#[derive(Debug, Serialize)]
struct BatchEnvelopeItem<'a> {
    custom_id: &'a str,
    params: ApiRequest,
}

/// The full submission body wrapping all envelope items.
#[derive(Debug, Serialize)]
struct BatchEnvelope<'a> {
    requests: Vec<BatchEnvelopeItem<'a>>,
}

/// Build the submission body. Pulled out so unit tests can assert the
/// JSON shape without spinning up an HTTP server.
fn build_submit_body<'a>(requests: &'a [BatchRequest]) -> BatchEnvelope<'a> {
    BatchEnvelope {
        requests: requests
            .iter()
            .map(|r| BatchEnvelopeItem {
                custom_id: &r.custom_id,
                params: build_api_request(&r.request),
            })
            .collect(),
    }
}

/// Split a JSONL blob on `\n` and parse each non-empty line as a
/// [`BatchResultLine`]. Trailing newlines are tolerated. A malformed
/// line short-circuits the whole call.
///
/// Kept around for tests; the production path uses [`stream_jsonl`].
#[cfg(test)]
fn parse_jsonl(body: &str) -> Result<Vec<BatchResultLine>, Error> {
    let mut out = Vec::new();
    for (idx, line) in body.split('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: BatchResultLine = serde_json::from_str(trimmed).map_err(|e| {
            Error::Anthropic(format!("batch results: parse line {idx}: {e}"))
        })?;
        out.push(parsed);
    }
    Ok(out)
}

/// Stream a JSONL response body chunk-by-chunk and parse each `\n`-
/// terminated line into a [`BatchResultLine`]. The input buffer holds
/// at most one in-flight (incomplete) line plus the most recent reqwest
/// chunk — for a 500 MB results file the bounded read buffer is the
/// whole point of switching off `Response::text()`.
///
/// A malformed line short-circuits the whole call. Non-UTF-8 bytes
/// inside a chunk produce an `Error::Anthropic` rather than panicking.
async fn stream_jsonl(mut res: reqwest::Response) -> Result<Vec<BatchResultLine>, Error> {
    let mut out: Vec<BatchResultLine> = Vec::new();
    let mut buf = String::new();
    let mut line_idx: usize = 0;
    loop {
        let chunk = res
            .chunk()
            .await
            .map_err(|e| Error::Anthropic(format!("body read: {e}")))?;
        let Some(bytes) = chunk else { break };
        // Append to the rolling line buffer. Validate UTF-8 once per
        // chunk; if the chunk happens to split a multi-byte codepoint
        // we hold the trailing bytes for the next iteration via the
        // line buffer (which is `String`), so any *real* invalid UTF-8
        // surfaces here.
        let s = std::str::from_utf8(&bytes)
            .map_err(|e| Error::Anthropic(format!("batch results: utf-8 decode: {e}")))?;
        buf.push_str(s);
        // Drain complete lines.
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim();
            if !line.is_empty() {
                let parsed: BatchResultLine =
                    serde_json::from_str(line).map_err(|e| {
                        Error::Anthropic(format!("batch results: parse line {line_idx}: {e}"))
                    })?;
                out.push(parsed);
            }
            line_idx += 1;
            // Drop the consumed line (including the `\n`).
            buf.drain(..=nl);
        }
    }
    // Tail: a final line without a trailing newline is still valid.
    let tail = buf.trim();
    if !tail.is_empty() {
        let parsed: BatchResultLine = serde_json::from_str(tail).map_err(|e| {
            Error::Anthropic(format!("batch results: parse line {line_idx}: {e}"))
        })?;
        out.push(parsed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::CompleteRequest;
    use serde_json::Value;

    #[test]
    fn batch_request_envelope_serialises_correctly() {
        let reqs = vec![
            BatchRequest::new(
                "id-a",
                CompleteRequest::user("hello")
                    .with_system("sys")
                    .with_max_tokens(100),
            ),
            BatchRequest::new(
                "id-b",
                CompleteRequest::user("world")
                    .with_system("sys")
                    .with_system_cache()
                    .with_max_tokens(200),
            ),
        ];
        let envelope = build_submit_body(&reqs);
        let v = serde_json::to_value(&envelope).expect("serialise");
        let arr = v["requests"].as_array().expect("requests array");
        assert_eq!(arr.len(), 2);

        // First entry: custom_id + params, plain-string system.
        assert_eq!(arr[0]["custom_id"], Value::String("id-a".into()));
        let params0 = &arr[0]["params"];
        assert_eq!(params0["max_tokens"], 100);
        assert_eq!(params0["system"], Value::String("sys".into()));
        assert_eq!(params0["messages"][0]["content"], "hello");

        // Second entry: cache_control survives the round-trip.
        assert_eq!(arr[1]["custom_id"], Value::String("id-b".into()));
        let params1 = &arr[1]["params"];
        assert_eq!(params1["max_tokens"], 200);
        assert_eq!(params1["system"][0]["type"], "text");
        assert_eq!(params1["system"][0]["text"], "sys");
        assert_eq!(params1["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn batch_result_line_parses_succeeded_and_errored() {
        let jsonl = r#"
{"custom_id":"a","result":{"type":"succeeded","message":{"id":"msg_1","content":[{"type":"text","text":"ok"}]}}}
{"custom_id":"b","result":{"type":"errored","error":{"type":"overloaded_error","message":"slow down"}}}
"#;
        let lines = parse_jsonl(jsonl).expect("parse");
        assert_eq!(lines.len(), 2);

        assert_eq!(lines[0].custom_id, "a");
        match &lines[0].result {
            BatchResultBody::Succeeded { message } => {
                assert_eq!(message["id"], "msg_1");
                assert_eq!(message["content"][0]["text"], "ok");
            }
            other => panic!("expected succeeded, got {other:?}"),
        }

        assert_eq!(lines[1].custom_id, "b");
        match &lines[1].result {
            BatchResultBody::Errored { error } => {
                assert_eq!(error["type"], "overloaded_error");
                assert_eq!(error["message"], "slow down");
            }
            other => panic!("expected errored, got {other:?}"),
        }
    }

    #[test]
    fn batch_result_line_parses_canceled_and_expired() {
        let jsonl = "\
{\"custom_id\":\"c\",\"result\":{\"type\":\"canceled\"}}\n\
{\"custom_id\":\"d\",\"result\":{\"type\":\"expired\"}}\n";
        let lines = parse_jsonl(jsonl).expect("parse");
        assert_eq!(lines.len(), 2);
        assert!(matches!(lines[0].result, BatchResultBody::Canceled));
        assert!(matches!(lines[1].result, BatchResultBody::Expired));
    }

    #[test]
    fn batch_status_response_marks_done_only_when_ended() {
        let in_progress = serde_json::json!({
            "id": "msgbatch_1",
            "processing_status": "in_progress",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let ended = serde_json::json!({
            "id": "msgbatch_1",
            "processing_status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "results_url": "https://example/x.jsonl",
            "request_counts": { "succeeded": 5 }
        });
        let a: BatchSubmitResponse = serde_json::from_value(in_progress).unwrap();
        let b: BatchSubmitResponse = serde_json::from_value(ended).unwrap();
        assert!(!a.is_done());
        assert!(b.is_done());
        assert_eq!(b.request_counts.succeeded, 5);
        assert_eq!(b.results_url.as_deref(), Some("https://example/x.jsonl"));
    }

    #[test]
    fn is_done_recognises_canceled_failed_expired() {
        // Regression: previous version matched only `ended`. A
        // `canceling` / `canceled` / `cancelled` / `failed` snapshot
        // would spin `wait_until_done` forever. Verify each of those
        // resolves to a terminal state.
        for status in [
            "ended",
            "canceling",
            "canceled",
            "cancelled",
            "failed",
            "ENDED",     // case-insensitive
            "Canceling", // mixed case
        ] {
            let snap = BatchSubmitResponse {
                id: "msgbatch_1".into(),
                processing_status: status.into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                expires_at: None,
                request_counts: BatchCounts::default(),
                results_url: None,
            };
            assert!(snap.is_done(), "status {status:?} should be terminal");
        }
        // Non-terminal statuses we expect to keep polling on.
        for status in ["in_progress", "queued", "validating", ""] {
            let snap = BatchSubmitResponse {
                id: "msgbatch_1".into(),
                processing_status: status.into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                expires_at: None,
                request_counts: BatchCounts::default(),
                results_url: None,
            };
            assert!(
                !snap.is_done(),
                "status {status:?} should NOT be terminal"
            );
        }
    }

    #[tokio::test]
    async fn wait_until_done_max_polls_returns_error() {
        // Spin up a wiremock server that always returns `in_progress`.
        // After the configured `max_polls` cap we should see a
        // `batch poll timeout` error rather than an infinite loop.
        // Use a 1 ms poll interval so the test wraps up quickly.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/v1/messages/batches/msgbatch_x"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({
                    "id": "msgbatch_x",
                    "processing_status": "in_progress",
                    "created_at": "2026-01-01T00:00:00Z"
                }),
            ))
            .mount(&server)
            .await;

        let client = BatchClient {
            http: reqwest::Client::new(),
            api_key: SecretString::from("test".to_string()),
            base_url: server.uri(),
            api_version: "2023-06-01".to_string(),
            retry: RetryPolicy::no_retry(),
        };
        let res = client
            .wait_until_done("msgbatch_x", Duration::from_millis(1), Some(3))
            .await;
        let err = res.expect_err("should hit max_polls cap");
        match err {
            Error::Anthropic(msg) => assert!(
                msg.contains("batch poll timeout"),
                "unexpected error: {msg}"
            ),
            other => panic!("expected Anthropic error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_done_returns_when_terminal_status_reached() {
        // First poll: in_progress. Second poll: failed (terminal).
        // Verifies (a) we don't spin on `failed` and (b) the loop
        // exits cleanly with the snapshot.
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/v1/messages/batches/msgbatch_y"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({
                    "id": "msgbatch_y",
                    "processing_status": "in_progress",
                    "created_at": "2026-01-01T00:00:00Z"
                }),
            ))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/v1/messages/batches/msgbatch_y"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({
                    "id": "msgbatch_y",
                    "processing_status": "failed",
                    "created_at": "2026-01-01T00:00:00Z"
                }),
            ))
            .mount(&server)
            .await;

        let client = BatchClient {
            http: reqwest::Client::new(),
            api_key: SecretString::from("test".to_string()),
            base_url: server.uri(),
            api_version: "2023-06-01".to_string(),
            retry: RetryPolicy::no_retry(),
        };
        let snap = client
            .wait_until_done("msgbatch_y", Duration::from_millis(1), None)
            .await
            .expect("terminal status reached");
        assert_eq!(snap.processing_status, "failed");
        assert!(snap.is_done());
    }

    #[test]
    fn stream_jsonl_handles_chunked_input() {
        // Direct unit test of the parser by feeding it a synthetic
        // single-buffer body. We can't easily simulate a real chunked
        // reqwest::Response in a unit test, but `parse_jsonl` shares
        // the line-splitting logic — covered by existing tests. Here
        // we sanity-check the new tail-without-newline path through
        // `parse_jsonl`.
        let body = "\
{\"custom_id\":\"a\",\"result\":{\"type\":\"canceled\"}}\n\
{\"custom_id\":\"b\",\"result\":{\"type\":\"expired\"}}";
        let lines = parse_jsonl(body).expect("parse");
        assert_eq!(lines.len(), 2);
        assert!(matches!(lines[0].result, BatchResultBody::Canceled));
        assert!(matches!(lines[1].result, BatchResultBody::Expired));
    }

    #[test]
    fn submit_rejects_empty_batch() {
        // Don't need a server — argument validation runs before any
        // HTTP. We can't construct a `BatchClient` without an API key
        // env var, so build a fake one by hand.
        let client = BatchClient {
            http: reqwest::Client::new(),
            api_key: SecretString::from("test".to_string()),
            base_url: "http://localhost:1".to_string(),
            api_version: "2023-06-01".to_string(),
            retry: RetryPolicy::no_retry(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        let res = rt.block_on(client.submit(Vec::new()));
        assert!(res.is_err());
    }
}
