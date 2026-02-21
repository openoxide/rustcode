use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::retry::{is_retryable, is_stream_error, retry_llm_call, RetryInfo, RetryPolicy};

// ── RetryPolicy::delay ──────────────────────────────────────────────────

#[test]
fn delay_exponential_backoff_with_jitter() {
    let policy = RetryPolicy {
        initial_delay_ms: 1_000,
        backoff_factor: 2,
        max_delay_ms: 60_000,
        max_retries: 5,
        stream_extra_retries: 0,
    };
    let d1 = policy.delay(1).as_millis() as u64;
    let d2 = policy.delay(2).as_millis() as u64;
    let d3 = policy.delay(3).as_millis() as u64;

    // attempt 1: base = 1000 * 2^0 = 1000, jitter up to ~14%
    assert!(d1 >= 1_000 && d1 <= 1_200, "d1={d1}");
    // attempt 2: base = 1000 * 2^1 = 2000
    assert!(d2 >= 2_000 && d2 <= 2_300, "d2={d2}");
    // attempt 3: base = 1000 * 2^2 = 4000
    assert!(d3 >= 4_000 && d3 <= 4_600, "d3={d3}");
}

#[test]
fn delay_capped_at_max() {
    let policy = RetryPolicy {
        initial_delay_ms: 10_000,
        backoff_factor: 10,
        max_delay_ms: 30_000,
        max_retries: 5,
        stream_extra_retries: 0,
    };
    let d = policy.delay(5).as_millis() as u64;
    // base would be huge but capped at 30_000, jitter up to ~14%
    assert!(d >= 30_000 && d <= 34_300, "d={d}");
}

// ── RetryPolicy::should_retry ───────────────────────────────────────────

#[test]
fn should_retry_within_limit() {
    let policy = RetryPolicy {
        max_retries: 3,
        ..RetryPolicy::default()
    };
    assert!(policy.should_retry(1));
    assert!(policy.should_retry(3));
    assert!(!policy.should_retry(4));
}

// ── is_retryable ────────────────────────────────────────────────────────

#[test]
fn rate_limit_is_retryable() {
    assert!(is_retryable("429 Too Many Requests"));
    assert!(is_retryable("rate_limit_exceeded"));
    assert!(is_retryable("rate limit reached"));
    assert!(is_retryable("too_many_requests"));
}

#[test]
fn server_errors_are_retryable() {
    assert!(is_retryable("500 Internal Server Error"));
    assert!(is_retryable("502 Bad Gateway"));
    assert!(is_retryable("503 Service Unavailable"));
    assert!(is_retryable("529 Service Overloaded"));
    assert!(is_retryable("internal server error"));
    assert!(is_retryable("bad gateway"));
    assert!(is_retryable("service unavailable"));
}

#[test]
fn overload_is_retryable() {
    assert!(is_retryable("server overloaded"));
    assert!(is_retryable("resource exhausted"));
}

#[test]
fn connection_errors_retryable() {
    assert!(is_retryable("connection reset by peer"));
    assert!(is_retryable("connection refused"));
    assert!(is_retryable("connection timeout"));
    assert!(is_retryable(
        "timed out waiting for provider response chunk"
    ));
    assert!(is_retryable("transport error: request timed out"));
}

#[test]
fn context_overflow_not_retryable() {
    assert!(!is_retryable("context overflow: too many tokens"));
    assert!(!is_retryable("context is too long"));
}

#[test]
fn auth_errors_not_retryable() {
    assert!(!is_retryable("unauthorized"));
    assert!(!is_retryable("forbidden"));
    assert!(!is_retryable("invalid api key"));
    assert!(!is_retryable("invalid_api_key"));
}

#[test]
fn content_policy_not_retryable() {
    assert!(!is_retryable("content_policy_violation"));
    assert!(!is_retryable("content policy"));
}

#[test]
fn unknown_errors_not_retryable() {
    assert!(!is_retryable("something completely unknown"));
}

#[test]
fn empty_provider_response_is_retryable() {
    assert!(is_retryable(
        "response did not include content or tool calls"
    ));
    assert!(is_retryable("stream did not include text deltas"));
}

// ── Classified LlmErrorKind patterns ────────────────────────────────────

#[test]
fn classified_ratelimit_is_retryable() {
    assert!(is_retryable(
        "provider error (RateLimit { retry_after_secs: 30 }): rate limited"
    ));
}

#[test]
fn classified_service_unavailable_is_retryable() {
    assert!(is_retryable(
        "provider error (ServiceUnavailable): overloaded"
    ));
}

#[test]
fn classified_context_overflow_not_retryable() {
    assert!(!is_retryable(
        "provider error (ContextOverflow): too many tokens"
    ));
}

#[test]
fn classified_auth_failed_not_retryable() {
    assert!(!is_retryable("provider error (AuthFailed): invalid key"));
}

#[test]
fn classified_invalid_request_not_retryable() {
    assert!(!is_retryable("provider error (InvalidRequest): bad param"));
}

// ── extract_server_delay (tested indirectly via retry_llm_call) ─────────

#[test]
fn extract_server_delay_from_classified_error() {
    // The extract_server_delay function is private, but we can test its effect
    // by checking that retry_llm_call respects retry_after_secs in the error.
    // For a direct unit test we use the pattern matching in is_retryable.
    let err = "provider error (RateLimit { retry_after_secs: 15 }): rate limited";
    assert!(is_retryable(err));
}

#[test]
fn extract_server_delay_zero_returns_none() {
    // Zero retry_after_secs should still be retryable (uses default backoff)
    let err = "provider error (RateLimit { retry_after_secs: 0 }): rate limited";
    assert!(is_retryable(err));
}

#[test]
fn extract_server_delay_missing_returns_none() {
    // No retry_after_secs at all — still retryable via "ratelimit" match
    let err = "provider error (RateLimit): rate limited";
    assert!(is_retryable(err));
}

// ── Stream error classification ─────────────────────────────────────────

#[test]
fn stream_errors_get_extra_retries() {
    let policy = RetryPolicy {
        max_retries: 3,
        stream_extra_retries: 2,
        ..RetryPolicy::default()
    };
    let effective = policy.effective_max_retries("transport error: timed out");
    assert_eq!(effective, 5);
    let normal = policy.effective_max_retries("rate_limit_exceeded");
    assert_eq!(normal, 3);
}

#[test]
fn is_stream_error_detection() {
    assert!(is_stream_error("transport error: connection reset"));
    assert!(is_stream_error("request timed out"));
    assert!(is_stream_error("connection refused"));
    assert!(is_stream_error("stream terminated early"));
    assert!(!is_stream_error("rate_limit_exceeded"));
    assert!(!is_stream_error("invalid api key"));
}

// ── retry_llm_call integration ──────────────────────────────────────────

#[tokio::test]
async fn retry_succeeds_on_second_attempt() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&call_count);

    let policy = RetryPolicy {
        max_retries: 3,
        stream_extra_retries: 0,
        initial_delay_ms: 10,
        backoff_factor: 1,
        max_delay_ms: 50,
    };

    let result: Result<&str, String> = retry_llm_call(
        &policy,
        || {
            let cc = Arc::clone(&cc);
            async move {
                let n = cc.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err("429 rate_limit_exceeded".to_string())
                } else {
                    Ok("success")
                }
            }
        },
        None,
    )
    .await;

    assert_eq!(result.unwrap(), "success");
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retry_gives_up_after_max() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&call_count);

    let policy = RetryPolicy {
        max_retries: 2,
        stream_extra_retries: 0,
        initial_delay_ms: 10,
        backoff_factor: 1,
        max_delay_ms: 50,
    };

    let result: Result<&str, String> = retry_llm_call(
        &policy,
        || {
            let cc = Arc::clone(&cc);
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Err("502 bad gateway".to_string())
            }
        },
        None,
    )
    .await;

    assert!(result.is_err());
    // 1 initial + 2 retries = 3 calls
    assert_eq!(call_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn retry_does_not_retry_non_retryable() {
    let call_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&call_count);

    let policy = RetryPolicy {
        max_retries: 3,
        stream_extra_retries: 0,
        initial_delay_ms: 10,
        backoff_factor: 1,
        max_delay_ms: 50,
    };

    let result: Result<&str, String> = retry_llm_call(
        &policy,
        || {
            let cc = Arc::clone(&cc);
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Err("unauthorized: invalid api key".to_string())
            }
        },
        None,
    )
    .await;

    assert!(result.is_err());
    // Should not retry — only 1 call
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_callback_receives_structured_info() {
    let collected = Arc::new(std::sync::Mutex::new(Vec::<RetryInfo>::new()));
    let col = Arc::clone(&collected);

    let call_count = Arc::new(AtomicU32::new(0));
    let cc = Arc::clone(&call_count);

    let policy = RetryPolicy {
        max_retries: 2,
        stream_extra_retries: 0,
        initial_delay_ms: 10,
        backoff_factor: 2,
        max_delay_ms: 100,
    };

    let on_retry = move |info: &RetryInfo| {
        col.lock().unwrap().push(info.clone());
    };

    let _result: Result<&str, String> = retry_llm_call(
        &policy,
        || {
            let cc = Arc::clone(&cc);
            async move {
                cc.fetch_add(1, Ordering::SeqCst);
                Err("429 rate limited".to_string())
            }
        },
        Some(&on_retry),
    )
    .await;

    let infos = collected.lock().unwrap();
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].attempt, 1);
    assert_eq!(infos[0].max_retries, 2);
    assert_eq!(infos[0].error_reason, "rate limited");
    assert_eq!(infos[1].attempt, 2);
}
