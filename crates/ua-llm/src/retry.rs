//! Async retry helper with exponential backoff + jitter.
//!
//! Wraps a fallible async operation and replays it on transient
//! failures (HTTP 429/5xx, transport timeouts, connection resets).
//! The classification is intentionally string-based: callers in this
//! crate fold transport / API errors into [`ua_core::Error::Anthropic`]
//! / [`ua_core::Error::Embedding`] strings, and [`is_retryable`]
//! pattern-matches on those.
//!
//! Used by [`crate::anthropic::AnthropicClient::complete`] and
//! [`crate::embeddings::OpenAiEmbeddings::embed`]. Both expose a
//! `with_retry_policy` builder so callers can override the defaults.

use std::future::Future;
use std::time::Duration;

use ua_core::Error;

/// Tunable retry parameters. The defaults are conservative enough for
/// the public Anthropic / OpenAI endpoints under typical load — five
/// attempts, 500 ms base delay, 30 s cap, jitter on.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Total attempts including the first try. `1` disables retry.
    pub max_attempts: u32,
    /// Initial backoff in milliseconds; doubled per retry.
    pub base_delay_ms: u64,
    /// Upper bound applied after exponential growth (and before jitter).
    pub max_delay_ms: u64,
    /// Multiply the delay by a random factor in `[0.5, 1.5]` (inclusive
    /// on both ends) to spread concurrent retriers.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Disable retries entirely. Useful in tests that want a single
    /// deterministic shot at a mock server.
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            base_delay_ms: 0,
            max_delay_ms: 0,
            jitter: false,
        }
    }

    fn delay_for(&self, attempt: u32) -> Duration {
        // attempt is 1-based: first retry uses base_delay_ms.
        let exp = attempt.saturating_sub(1).min(20); // guard against shifting too far
        let raw = self.base_delay_ms.saturating_mul(1u64 << exp);
        let capped = raw.min(self.max_delay_ms);
        let final_ms = if self.jitter && capped > 0 {
            // Inclusive range so the doc-comment range matches the
            // sampled values: `[0.5, 1.5]` rather than `[0.5, 1.5)`.
            let factor: f64 = rand::random_range(0.5..=1.5);
            ((capped as f64) * factor) as u64
        } else {
            capped
        };
        Duration::from_millis(final_ms)
    }
}

/// Returns `true` for errors that look like transient HTTP / transport
/// failures. Conservative on purpose: anything we can't confidently
/// classify is treated as fatal.
///
/// Classification is anchored to the `status=NNN` token that callers
/// inject into error strings (see `anthropic.rs` / `embeddings.rs`):
///
/// * Retry: `408`, `429`, any `5xx`, plus the Cloudflare-specific
///   `521`, `522`, `524` (already covered by the `5xx` check but kept
///   explicit so the docs name them).
/// * Never retry: any other 4xx (auth/validation are not transient,
///   even when their server-side body happens to contain words like
///   "timeout" or "connection reset"). This is the regression fix for
///   issue #1 of the retry review.
/// * Transport-level errors (no `status=` token in the message) fall
///   through to a substring match on known transport keywords.
pub fn is_retryable(err: &Error) -> bool {
    let msg = match err {
        Error::Anthropic(m) | Error::Embedding(m) => m.as_str(),
        _ => return false,
    };
    if let Some(code) = extract_status_code(msg) {
        // We have a server-assigned status code. Decide purely on the
        // numeric value — never let server-controlled body text flip
        // a 4xx into a retry.
        return is_retryable_status(code);
    }
    // No status token: this is a transport-layer error. Match against
    // the prefixes our HTTP wrappers emit ("http: error sending
    // request", "body read: …") plus a handful of well-known transport
    // keywords. Body-read failures are treated as transient because
    // they typically indicate a mid-stream disconnect.
    let lower = msg.to_ascii_lowercase();
    lower.contains("http: error sending request")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("connection refused")
        || lower.contains("body read")
}

/// Extract the numeric status code following a `status=` token, if any.
/// Returns `None` when the token is absent or the value can't be parsed
/// as a 3-digit number.
fn extract_status_code(msg: &str) -> Option<u16> {
    let idx = msg.find("status=")?;
    let rest = &msg[idx + "status=".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok()
}

/// Decide whether a given HTTP status code should trigger a retry.
/// Pulled out so tests (and callers that already have a numeric code)
/// can exercise the policy directly.
fn is_retryable_status(code: u16) -> bool {
    match code {
        // Request Timeout — RFC 9110 §15.5.9. Some CDNs surface this
        // when an upstream hop times out reading the request body.
        408 => true,
        // Too Many Requests.
        429 => true,
        // 5xx — every server-side error is at least worth one replay.
        // Cloudflare-specific 521 / 522 / 524 fall into this range and
        // are common when fronting Anthropic / OpenAI traffic through
        // Cloudflare.
        500..=599 => true,
        // Anything else (1xx/2xx/3xx/other 4xx) is *not* transient.
        // Crucially, 401/403/400 with a body containing "timeout" or
        // "connection reset" must NOT retry.
        _ => false,
    }
}

/// Maximum number of characters of an error message we'll write to the
/// retry log line. Beyond this we truncate. Anthropic/OpenAI sometimes
/// echo the offending request back inside their error JSON, which can
/// run to tens of kilobytes — keep the log readable.
const LOG_REASON_MAX_LEN: usize = 200;

/// Sanitise a server-controlled error string for safe inclusion in a
/// log line. Truncates to [`LOG_REASON_MAX_LEN`] and replaces ASCII
/// control characters (newlines, tabs, ESC, …) with `?`. Defends
/// against log-injection where a 4xx body containing CRLF could forge
/// extra log records. A trailing `…` is appended when truncation
/// happened so readers can see the message was cut.
fn sanitize_for_log(s: &str) -> String {
    let mut truncated = false;
    // Use `char_indices` so multi-byte UTF-8 is never split mid-codepoint.
    let cut = s
        .char_indices()
        .nth(LOG_REASON_MAX_LEN)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    if cut < s.len() {
        truncated = true;
    }
    let head = &s[..cut];
    let mut out = String::with_capacity(head.len() + 1);
    for c in head.chars() {
        if c.is_control() {
            out.push('?');
        } else {
            out.push(c);
        }
    }
    if truncated {
        out.push('…');
    }
    out
}

/// Run `op` up to `policy.max_attempts` times, sleeping between
/// attempts on retryable failures. Non-retryable errors short-circuit.
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, mut op: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    let max = policy.max_attempts.max(1);
    let mut attempt: u32 = 1;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempt >= max || !is_retryable(&e) {
                    return Err(e);
                }
                let delay = policy.delay_for(attempt);
                let reason = sanitize_for_log(&e.to_string());
                tracing::warn!(
                    attempt,
                    max,
                    backoff_ms = delay.as_millis() as u64,
                    reason = %reason,
                    "llm request failed, retrying"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn retryable() -> Error {
        Error::Anthropic("anthropic api: status=503 body=overloaded".into())
    }

    fn fatal() -> Error {
        Error::Anthropic("anthropic api: status=400 body=bad request".into())
    }

    fn fast_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 4,
            base_delay_ms: 1,
            max_delay_ms: 4,
            jitter: false,
        }
    }

    #[test]
    fn classifies_statuses() {
        assert!(is_retryable(&retryable()));
        assert!(is_retryable(&Error::Embedding(
            "openai api: status=429 body=slow down".into()
        )));
        assert!(is_retryable(&Error::Anthropic(
            "http: error sending request: connection reset".into()
        )));
        assert!(!is_retryable(&fatal()));
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_inner = calls.clone();
        let policy = fast_policy();
        let res: Result<u32, Error> = with_retry(&policy, move || {
            let calls = calls_inner.clone();
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 3 {
                    Err(retryable())
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn fatal_short_circuits() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_inner = calls.clone();
        let policy = fast_policy();
        let res: Result<(), Error> = with_retry(&policy, move || {
            let calls = calls_inner.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(fatal())
            }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn exhausts_attempts() {
        let calls = Arc::new(AtomicU32::new(0));
        let calls_inner = calls.clone();
        let policy = fast_policy();
        let res: Result<(), Error> = with_retry(&policy, move || {
            let calls = calls_inner.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(retryable())
            }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), policy.max_attempts);
    }

    #[test]
    fn is_retryable_does_not_retry_401_with_timeout_in_body() {
        // Regression for issue #1 of the retry review: a 401 whose
        // server-controlled body happens to contain transport-keyword
        // strings ("timeout", "connection reset", "body read") must
        // NOT be retried. Auth failures are not transient.
        let cases = [
            "anthropic api: status=401 body=request timed out upstream",
            "anthropic api: status=401 body=connection reset by peer",
            "openai api: status=403 body=body read failed mid-stream",
            "anthropic api: status=400 body=timeout",
        ];
        for msg in cases {
            assert!(
                !is_retryable(&Error::Anthropic(msg.into())),
                "should not retry 4xx with body keywords: {msg}"
            );
        }
    }

    #[test]
    fn is_retryable_retries_408_521_522_524() {
        // 408 (Request Timeout) and the Cloudflare-family codes 521 /
        // 522 / 524 are transient; verify they all replay.
        for code in [408u16, 521, 522, 524] {
            let msg = format!("anthropic api: status={code} body=cf upstream issue");
            assert!(
                is_retryable(&Error::Anthropic(msg.clone())),
                "should retry status {code}: {msg}"
            );
            assert!(is_retryable_status(code));
        }
    }

    #[test]
    fn is_retryable_status_helper_classification() {
        // Sanity: the numeric helper agrees with the spec.
        for code in [408u16, 429, 500, 502, 503, 504, 521, 522, 524, 599] {
            assert!(is_retryable_status(code), "should retry {code}");
        }
        for code in [200u16, 301, 400, 401, 403, 404, 410, 422] {
            assert!(!is_retryable_status(code), "should NOT retry {code}");
        }
    }

    #[test]
    fn extract_status_code_parses_token() {
        assert_eq!(
            extract_status_code("anthropic api: status=503 body=overloaded"),
            Some(503)
        );
        assert_eq!(
            extract_status_code("openai api: status=429 body=slow"),
            Some(429)
        );
        // No token at all → None.
        assert_eq!(extract_status_code("http: error sending request"), None);
        // Token without digits → None.
        assert_eq!(extract_status_code("status= "), None);
    }

    #[test]
    fn jitter_inclusive_endpoints_within_bounds() {
        // The doc-comment says jitter is `[0.5, 1.5]` (inclusive). The
        // implementation now uses `0.5..=1.5`. We can't assert the
        // upper bound is *hit*, but we can at least sample a few times
        // and confirm every produced delay falls in the documented
        // window. base_delay_ms × 0.5 ≤ delay ≤ base_delay_ms × 1.5.
        let p = RetryPolicy {
            max_attempts: 10,
            base_delay_ms: 1000,
            max_delay_ms: 1000,
            jitter: true,
        };
        for _ in 0..100 {
            let d = p.delay_for(1).as_millis() as u64;
            assert!(d >= 500, "jitter floor breached: {d}");
            assert!(d <= 1500, "jitter ceiling breached: {d}");
        }
    }

    #[test]
    fn sanitize_for_log_truncates_long_messages() {
        // 600-char message — well above the 200-char cap.
        let long = "a".repeat(600);
        let out = sanitize_for_log(&long);
        // ASCII so byte length == char count - 1 (for the trailing …).
        // Actual char count = 200 + 1 ('…').
        let chars = out.chars().count();
        assert_eq!(chars, 201, "expected 200 chars + ellipsis, got {chars}");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn sanitize_for_log_replaces_control_chars() {
        // Embedded CRLF + ESC: classic log-injection payload.
        let evil = "x\r\nFAKE LOG LINE\x1b[31mred\x1b[0m end";
        let out = sanitize_for_log(evil);
        // No control chars survive.
        assert!(
            !out.chars().any(|c| c.is_control()),
            "control char leaked into log: {out:?}"
        );
        // Visible chars are preserved.
        assert!(out.contains("FAKE LOG LINE"));
        assert!(out.contains("red"));
    }

    #[test]
    fn sanitize_for_log_handles_short_messages_unchanged() {
        let s = "boring 503 body";
        assert_eq!(sanitize_for_log(s), s);
    }

    #[test]
    fn sanitize_for_log_does_not_split_multibyte() {
        // 250 chars of a 4-byte codepoint. Truncation must land on a
        // codepoint boundary or `&s[..cut]` panics.
        let crab = "\u{1F980}".repeat(250); // 🦀 × 250
        let out = sanitize_for_log(&crab);
        // Should have truncated cleanly with the ellipsis appended.
        assert!(out.ends_with('…'));
        // 200 crabs + ellipsis = 201 chars.
        assert_eq!(out.chars().count(), 201);
    }

    #[test]
    fn delay_caps_and_grows() {
        let p = RetryPolicy {
            max_attempts: 10,
            base_delay_ms: 100,
            max_delay_ms: 800,
            jitter: false,
        };
        assert_eq!(p.delay_for(1), Duration::from_millis(100));
        assert_eq!(p.delay_for(2), Duration::from_millis(200));
        assert_eq!(p.delay_for(3), Duration::from_millis(400));
        assert_eq!(p.delay_for(4), Duration::from_millis(800));
        // Capped.
        assert_eq!(p.delay_for(5), Duration::from_millis(800));
        assert_eq!(p.delay_for(20), Duration::from_millis(800));
    }
}
