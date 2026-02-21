// ── LLM retry logic ──────────────────────────────────────────────────
//
// Provides automatic retry with exponential backoff for transient LLM
// failures (rate limits, overloads, server errors).

use std::time::Duration;

/// Initial delay before first retry (2 seconds).
const RETRY_INITIAL_DELAY_MS: u64 = 2_000;

/// Backoff multiplier between retries.
const RETRY_BACKOFF_FACTOR: u64 = 2;

/// Maximum delay between retries (30 seconds).
const MAX_DELAY_MS: u64 = 30_000;

/// Default maximum number of retry attempts.
const DEFAULT_MAX_RETRIES: u32 = 3;

/// Extra retries granted for stream/transport errors (on top of `max_retries`).
const DEFAULT_STREAM_EXTRA_RETRIES: u32 = 2;

/// Retry policy configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum retries for non-stream errors.
    pub max_retries: u32,
    /// Extra retries for stream/transport errors (total = `max_retries` + `stream_extra_retries`).
    pub stream_extra_retries: u32,
    pub initial_delay_ms: u64,
    pub backoff_factor: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            stream_extra_retries: DEFAULT_STREAM_EXTRA_RETRIES,
            initial_delay_ms: RETRY_INITIAL_DELAY_MS,
            backoff_factor: RETRY_BACKOFF_FACTOR,
            max_delay_ms: MAX_DELAY_MS,
        }
    }
}

impl RetryPolicy {
    /// Calculate the delay before the next retry attempt.
    ///
    /// Uses exponential backoff with ~15% jitter:
    /// `initial_delay * backoff_factor^(attempt - 1) + jitter`, capped at `max_delay_ms`.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let base = self.initial_delay_ms
            * self
                .backoff_factor
                .saturating_pow(attempt.saturating_sub(1));
        let capped = base.min(self.max_delay_ms);
        // Add ~15% jitter using nanosecond clock to avoid thundering herd
        let jitter_range = capped / 7; // ~14%
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()))
            .unwrap_or(0);
        let jitter = if jitter_range > 0 {
            nanos % jitter_range
        } else {
            0
        };
        Duration::from_millis(capped.saturating_add(jitter))
    }

    /// Effective max retries for the given error type.
    ///
    /// Stream/transport errors get additional retries.
    #[must_use]
    pub fn effective_max_retries(&self, error_msg: &str) -> u32 {
        if is_stream_error(error_msg) {
            self.max_retries + self.stream_extra_retries
        } else {
            self.max_retries
        }
    }

    /// Check if more retries are allowed for the given attempt and error.
    #[must_use]
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt <= self.max_retries
    }
}

/// Structured information about a retry attempt, passed to the `on_retry` callback.
#[derive(Debug, Clone)]
pub struct RetryInfo {
    /// Current attempt number (1-based).
    pub attempt: u32,
    /// Maximum retries configured for this error type.
    pub max_retries: u32,
    /// Delay before the next attempt.
    pub delay: Duration,
    /// Human-readable error reason that triggered the retry.
    pub error_reason: String,
}

/// Determines if an LLM error is retryable.
///
/// Checks both the [`LlmErrorKind`] typed classification (via the Debug
/// format produced by `LlmError::Classified`) and raw string patterns for
/// unclassified transport errors.
///
/// Retryable errors include:
/// - Classified: `LlmErrorKind::RateLimit`, `LlmErrorKind::ServiceUnavailable`
/// - Rate limiting (429, "`rate_limit`", "`too_many_requests`")
/// - Server overload ("overloaded", "unavailable")
/// - Transient server errors (500, 502, 503, 529)
///
/// Non-retryable errors include:
/// - Classified: `LlmErrorKind::ContextOverflow`, `AuthFailed`, `InvalidRequest`
/// - Context overflow (compaction handles this)
/// - Authentication errors (401, 403)
/// - Invalid request errors (400)
/// - Content policy violations
#[must_use]
pub fn is_retryable(error_msg: &str) -> bool {
    let lower = error_msg.to_lowercase();

    // ── Typed classification (LlmError::Classified display format) ────────
    // Display: "provider error (RateLimit { ... }): ..."
    // Debug identifiers appear lowercased in the display string.
    if lower.contains("ratelimit") {
        return true;
    }
    if lower.contains("serviceunavailable") {
        return true;
    }
    if lower.contains("contextoverflow")
        || lower.contains("authfailed")
        || lower.contains("invalidrequest")
    {
        return false;
    }

    // ── String-pattern fallback (unclassified Transport errors) ──────────

    // Not retryable: context overflow — handled by compaction
    if lower.contains("context") && (lower.contains("overflow") || lower.contains("too long")) {
        return false;
    }

    // Not retryable: auth errors
    if lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
    {
        return false;
    }

    // Not retryable: content policy
    if lower.contains("content_policy") || lower.contains("content policy") {
        return false;
    }

    // Retryable: rate limits
    if lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("too_many_requests")
        || lower.contains("429")
    {
        return true;
    }

    // Retryable: overloaded / unavailable
    if lower.contains("overloaded") || lower.contains("unavailable") || lower.contains("exhausted")
    {
        return true;
    }

    // Retryable: empty / invalid response from provider (often transient with proxies like
    // OpenRouter — the upstream model occasionally returns nothing and retrying usually succeeds)
    if lower.contains("did not include content or tool calls")
        || lower.contains("stream did not include text deltas")
    {
        return true;
    }

    // Retryable: server errors
    if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("529")
        || lower.contains("internal server error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
    {
        return true;
    }

    // Retryable: connection errors
    if lower.contains("connection")
        && (lower.contains("reset") || lower.contains("refused") || lower.contains("timeout"))
    {
        return true;
    }
    if lower.contains("timed out waiting for provider response chunk")
        || (lower.contains("transport error") && lower.contains("timed out"))
    {
        return true;
    }

    false
}

/// Determines if an error is a stream/transport error (eligible for extra retries).
#[must_use]
pub fn is_stream_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("transport")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("stream")
}

/// Extract a server-suggested retry delay from a classified `RateLimit` error.
///
/// Parses the `retry_after_secs: N` field from the `LlmError::Classified` Display
/// format (e.g. `"provider error (RateLimit { retry_after_secs: 30 }): ..."`).
#[must_use]
fn extract_server_delay(err_msg: &str) -> Option<Duration> {
    let marker = "retry_after_secs: ";
    let pos = err_msg.find(marker)?;
    let after = &err_msg[pos + marker.len()..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    let secs: u64 = digits.parse().ok()?;
    if secs > 0 {
        Some(Duration::from_secs(secs))
    } else {
        None
    }
}

/// Summarise an LLM error message into a short reason string for display.
fn summarise_error(err_msg: &str) -> String {
    let lower = err_msg.to_lowercase();
    if lower.contains("ratelimit") || lower.contains("rate_limit") || lower.contains("429") {
        return "rate limited".to_string();
    }
    if lower.contains("timed out") {
        return "stream timed out".to_string();
    }
    if lower.contains("connection") {
        return "connection error".to_string();
    }
    if lower.contains("overloaded") || lower.contains("serviceunavailable") {
        return "provider overloaded".to_string();
    }
    if lower.contains("503") || lower.contains("502") || lower.contains("500") {
        return "server error".to_string();
    }
    // Fallback: first 60 chars
    let short: String = err_msg.chars().take(60).collect();
    if short.len() < err_msg.len() {
        format!("{short}…")
    } else {
        short
    }
}

/// Execute an async LLM call with retry logic.
///
/// Retries the closure up to `policy.max_retries` times on retryable errors
/// (with extra retries for stream/transport errors), using exponential backoff
/// with jitter. Server-suggested delays (from `retry_after_secs`) are honoured.
///
/// The optional `on_retry` callback receives structured [`RetryInfo`] before
/// each retry delay so the TUI can display retry progress in the activity panel.
pub async fn retry_llm_call<F, Fut, T, E>(
    policy: &RetryPolicy,
    mut make_call: F,
    on_retry: Option<&(dyn Fn(&RetryInfo) + Send + Sync)>,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        match make_call().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let err_msg = err.to_string();
                let effective_max = policy.effective_max_retries(&err_msg);

                if !is_retryable(&err_msg) || attempt > effective_max {
                    return Err(err);
                }

                // Prefer server-suggested delay, otherwise use exponential backoff
                let delay = extract_server_delay(&err_msg).unwrap_or_else(|| policy.delay(attempt));

                tracing::warn!(
                    attempt,
                    effective_max,
                    delay_ms = delay.as_millis() as u64,
                    error = %err_msg,
                    "LLM call failed, retrying"
                );
                if let Some(notify) = on_retry {
                    notify(&RetryInfo {
                        attempt,
                        max_retries: effective_max,
                        delay,
                        error_reason: summarise_error(&err_msg),
                    });
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
}
