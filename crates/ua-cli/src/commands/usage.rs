//! LLM token-usage aggregation + dollar estimate.
//!
//! Used by `analyze --with-llm` to roll up the per-call
//! [`ua_llm::TokenUsage`] payloads and print one summary line at the
//! end of a run. Pricing constants are best-effort and live here so a
//! `--llm-model` override is classified by *prefix* — version bumps
//! within a tier (`claude-opus-4-7` → `claude-opus-4-8`) keep the same
//! rate without code edits.
//!
//! Pricing as of 2026-01 (anthropic.com/pricing):
//!  * opus class:   input $15 / Mtok, output $75 / Mtok,
//!    cache_write $18.75 / Mtok, cache_read $1.50 / Mtok
//!  * sonnet class: input $3 / Mtok, output $15 / Mtok,
//!    cache_write $3.75 / Mtok, cache_read $0.30 / Mtok
//!  * haiku class:  input $0.80 / Mtok, output $4 / Mtok,
//!    cache_write $1 / Mtok, cache_read $0.08 / Mtok
//!
//! Unknown models fall back to opus rates and emit a `tracing::debug!`
//! so misclassifications don't go unnoticed.

/// Aggregated token usage across one analyze run.
#[derive(Default, Debug, Clone, Copy)]
pub struct TokenTotals {
    /// Sum of `input_tokens` reported across every successful call.
    pub input: u64,
    /// Sum of `output_tokens`.
    pub output: u64,
    /// Sum of `cache_creation_input_tokens` — written to a fresh
    /// ephemeral cache block.
    pub cache_create: u64,
    /// Sum of `cache_read_input_tokens` — served from an existing
    /// cache block. Billed at ~10% of the input rate.
    pub cache_read: u64,
}

impl TokenTotals {
    /// Fold one call's [`ua_llm::TokenUsage`] into the running total.
    pub fn add(&mut self, u: &ua_llm::TokenUsage) {
        self.input += u.input_tokens as u64;
        self.output += u.output_tokens as u64;
        self.cache_create += u.cache_creation_input_tokens as u64;
        self.cache_read += u.cache_read_input_tokens as u64;
    }

    /// `true` when no call ever landed (every file came from cache or
    /// every call failed before reporting usage).
    pub fn is_zero(&self) -> bool {
        self.input == 0 && self.output == 0 && self.cache_create == 0 && self.cache_read == 0
    }

    /// Rough cost in USD for `model`. See module docs for the rate
    /// table; unknown models fall back to opus-tier pricing and log a
    /// `debug!` line.
    pub fn estimate_usd(&self, model: &str) -> f64 {
        let rates = rates_for(model);
        let m = 1_000_000.0_f64;
        (self.input as f64) * rates.input / m
            + (self.output as f64) * rates.output / m
            + (self.cache_create as f64) * rates.cache_write / m
            + (self.cache_read as f64) * rates.cache_read / m
    }
}

/// Per-million-token USD rates for one model tier.
#[derive(Debug, Clone, Copy)]
struct Rates {
    input: f64,
    output: f64,
    cache_write: f64,
    cache_read: f64,
}

const OPUS: Rates = Rates {
    input: 15.0,
    output: 75.0,
    cache_write: 18.75,
    cache_read: 1.50,
};

const SONNET: Rates = Rates {
    input: 3.0,
    output: 15.0,
    cache_write: 3.75,
    cache_read: 0.30,
};

const HAIKU: Rates = Rates {
    input: 0.80,
    output: 4.0,
    cache_write: 1.0,
    cache_read: 0.08,
};

/// Pick the rate table by model-name prefix. Anthropic's naming
/// scheme is `claude-{tier}-{major}-{minor}[-suffix]`; matching by
/// prefix means a future `claude-opus-4-8` is still classified
/// correctly.
fn rates_for(model: &str) -> Rates {
    if model.starts_with("claude-opus") {
        OPUS
    } else if model.starts_with("claude-sonnet") {
        SONNET
    } else if model.starts_with("claude-haiku") {
        HAIKU
    } else {
        // Don't crash, but make it loud enough to spot in logs. Falling
        // back to opus rates rather than the cheaper tiers is the
        // conservative choice — we'd rather over-report a budget than
        // lull a user into thinking a run cost less than it did.
        tracing::debug!(
            target: "ua_cli::usage",
            model = %model,
            "unknown model — falling back to opus pricing for cost estimate"
        );
        OPUS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact arithmetic: 1M input opus tokens = $15, 1M output = $75 ⇒ $90.
    #[test]
    fn opus_one_million_each() {
        let t = TokenTotals {
            input: 1_000_000,
            output: 1_000_000,
            cache_create: 0,
            cache_read: 0,
        };
        let usd = t.estimate_usd("claude-opus-4-7");
        assert!((usd - 90.0).abs() < 1e-9, "got {usd}");
    }

    /// Sonnet: 1M input + 1M output = $3 + $15 = $18.
    #[test]
    fn sonnet_one_million_each() {
        let t = TokenTotals {
            input: 1_000_000,
            output: 1_000_000,
            cache_create: 0,
            cache_read: 0,
        };
        let usd = t.estimate_usd("claude-sonnet-4-6");
        assert!((usd - 18.0).abs() < 1e-9, "got {usd}");
    }

    /// Haiku: 1M input + 1M output = $0.80 + $4 = $4.80.
    #[test]
    fn haiku_one_million_each() {
        let t = TokenTotals {
            input: 1_000_000,
            output: 1_000_000,
            cache_create: 0,
            cache_read: 0,
        };
        let usd = t.estimate_usd("claude-haiku-4-5");
        assert!((usd - 4.80).abs() < 1e-9, "got {usd}");
    }

    /// Cache rates: 1M cache_read on opus = $1.50.
    #[test]
    fn opus_cache_read_only() {
        let t = TokenTotals {
            input: 0,
            output: 0,
            cache_create: 0,
            cache_read: 1_000_000,
        };
        let usd = t.estimate_usd("claude-opus-4-7");
        assert!((usd - 1.50).abs() < 1e-9, "got {usd}");
    }

    /// Unknown model falls back to opus rates — same answer as the
    /// opus fixture above.
    #[test]
    fn unknown_model_uses_opus_fallback() {
        let t = TokenTotals {
            input: 1_000_000,
            output: 0,
            cache_create: 0,
            cache_read: 0,
        };
        let usd = t.estimate_usd("gpt-5-turbo");
        assert!((usd - 15.0).abs() < 1e-9, "got {usd}");
    }

    #[test]
    fn add_folds_usage() {
        let mut t = TokenTotals::default();
        t.add(&ua_llm::TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 10,
            cache_read_input_tokens: 5,
        });
        t.add(&ua_llm::TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_input_tokens: 3,
            cache_read_input_tokens: 4,
        });
        assert_eq!(t.input, 101);
        assert_eq!(t.output, 52);
        assert_eq!(t.cache_create, 13);
        assert_eq!(t.cache_read, 9);
        assert!(!t.is_zero());
    }

    #[test]
    fn is_zero_default() {
        let t = TokenTotals::default();
        assert!(t.is_zero());
    }
}
