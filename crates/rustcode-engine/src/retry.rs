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

/// Retry policy configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub backoff_factor: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay_ms: RETRY_INITIAL_DELAY_MS,
            backoff_factor: RETRY_BACKOFF_FACTOR,
            max_delay_ms: MAX_DELAY_MS,
        }
    }
}

impl RetryPolicy {
    /// Calculate the delay before the next retry attempt.
    ///
    /// Uses exponential backoff: `initial_delay * backoff_factor^(attempt - 1)`,
    /// capped at `max_delay_ms`.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let delay = self.initial_delay_ms
            * self
                .backoff_factor
                .saturating_pow(attempt.saturating_sub(1));
        Duration::from_millis(delay.min(self.max_delay_ms))
    }

    /// Check if more retries are allowed for the given attempt number.
    #[must_use]
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt <= self.max_retries
    }
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

    false
}

/// Execute an async LLM call with retry logic.
///
/// Retries the closure up to `policy.max_retries` times on retryable errors,
/// with exponential backoff between attempts.
pub async fn retry_llm_call<F, Fut, T, E>(policy: &RetryPolicy, mut make_call: F) -> Result<T, E>
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

                if !is_retryable(&err_msg) || !policy.should_retry(attempt) {
                    return Err(err);
                }

                let delay = policy.delay(attempt);
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = %err_msg,
                    "LLM call failed, retrying"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_exponential_backoff() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1), Duration::from_millis(2_000));
        assert_eq!(policy.delay(2), Duration::from_millis(4_000));
        assert_eq!(policy.delay(3), Duration::from_millis(8_000));
    }

    #[test]
    fn delay_capped_at_max() {
        let policy = RetryPolicy {
            max_delay_ms: 5_000,
            ..Default::default()
        };
        // attempt 3: 2000 * 2^2 = 8000 -> capped at 5000
        assert_eq!(policy.delay(3), Duration::from_millis(5_000));
    }

    #[test]
    fn should_retry_within_limit() {
        let policy = RetryPolicy::default(); // max_retries = 3
        assert!(policy.should_retry(1));
        assert!(policy.should_retry(2));
        assert!(policy.should_retry(3));
        assert!(!policy.should_retry(4));
    }

    #[test]
    fn rate_limit_is_retryable() {
        assert!(is_retryable("429 Too Many Requests"));
        assert!(is_retryable("rate_limit_exceeded"));
        assert!(is_retryable("Rate limit reached"));
        assert!(is_retryable("too_many_requests"));
    }

    #[test]
    fn server_errors_are_retryable() {
        assert!(is_retryable("500 Internal Server Error"));
        assert!(is_retryable("502 Bad Gateway"));
        assert!(is_retryable("503 Service Unavailable"));
        assert!(is_retryable("529 overloaded"));
    }

    #[test]
    fn overload_is_retryable() {
        assert!(is_retryable("Overloaded"));
        assert!(is_retryable("Server unavailable"));
        assert!(is_retryable("Resource exhausted"));
    }

    #[test]
    fn connection_errors_retryable() {
        assert!(is_retryable("connection reset by peer"));
        assert!(is_retryable("connection refused"));
        assert!(is_retryable("connection timeout"));
    }

    #[test]
    fn context_overflow_not_retryable() {
        assert!(!is_retryable("context overflow: too many tokens"));
        assert!(!is_retryable("context length is too long"));
    }

    #[test]
    fn auth_errors_not_retryable() {
        assert!(!is_retryable("401 Unauthorized"));
        assert!(!is_retryable("403 Forbidden"));
        assert!(!is_retryable("Invalid API key"));
        assert!(!is_retryable("invalid_api_key"));
    }

    #[test]
    fn content_policy_not_retryable() {
        assert!(!is_retryable("content_policy_violation"));
        assert!(!is_retryable("content policy error"));
    }

    #[test]
    fn unknown_errors_not_retryable() {
        assert!(!is_retryable("something completely different"));
        assert!(!is_retryable("parsing error: invalid json"));
    }

    // Tests for LlmErrorKind typed classification (via Display/Debug format)
    #[test]
    fn classified_ratelimit_is_retryable() {
        // LlmError::Classified { kind: LlmErrorKind::RateLimit, ... } displays as:
        // "provider error (RateLimit { retry_after_secs: 0 }): provider returned 429: ..."
        assert!(is_retryable(
            "provider error (RateLimit { retry_after_secs: 0 }): provider returned 429: too many"
        ));
    }

    #[test]
    fn classified_service_unavailable_is_retryable() {
        assert!(is_retryable(
            "provider error (ServiceUnavailable): provider returned 503: overloaded"
        ));
    }

    #[test]
    fn classified_context_overflow_not_retryable() {
        assert!(!is_retryable(
            "provider error (ContextOverflow): provider returned 400: context too long"
        ));
    }

    #[test]
    fn classified_auth_failed_not_retryable() {
        assert!(!is_retryable(
            "provider error (AuthFailed): provider returned 401: unauthorized"
        ));
    }

    #[test]
    fn classified_invalid_request_not_retryable() {
        assert!(!is_retryable(
            "provider error (InvalidRequest): provider returned 400: bad request"
        ));
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);
        let policy = RetryPolicy {
            initial_delay_ms: 1, // 1ms for fast tests
            ..Default::default()
        };

        let result: Result<&str, String> = retry_llm_call(&policy, || {
            let count = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if count == 0 {
                    Err("429 rate_limit_exceeded".to_string())
                } else {
                    Ok("success")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_gives_up_after_max() {
        let policy = RetryPolicy {
            max_retries: 2,
            initial_delay_ms: 1,
            ..Default::default()
        };

        let result: Result<(), String> = retry_llm_call(&policy, || async {
            Err::<(), String>("503 Service Unavailable".to_string())
        })
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_does_not_retry_non_retryable() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);
        let policy = RetryPolicy {
            initial_delay_ms: 1,
            ..Default::default()
        };

        let result: Result<(), String> = retry_llm_call(&policy, || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), String>("401 Unauthorized".to_string()) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "should not retry auth errors"
        );
    }
}
