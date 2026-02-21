use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Value,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub initiator: RequestInitiator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestInitiator {
    User,
    Agent,
}

/// Token usage statistics from an LLM API response.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Tokens consumed by the input (prompt + messages).
    pub input: u64,
    /// Tokens generated in the output.
    pub output: u64,
    /// Total tokens (input + output). May differ from sum if provider reports differently.
    pub total: u64,
    /// Tokens read from cache (prompt caching).
    pub cache_read: u64,
    /// Tokens written to cache.
    pub cache_write: u64,
}

/// Response from an LLM chat completion.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// The assistant's text reply (may be empty if only tool calls).
    pub text: String,
    /// Optional model reasoning/thinking text for this turn.
    pub reasoning: Option<String>,
    /// Incremental reasoning chunks when provider emits deltas.
    pub reasoning_chunks: Vec<String>,
    /// Tool calls requested by the assistant.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage statistics, if reported by the provider.
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub chunks: Vec<String>,
}

/// Structured classification of LLM provider errors.
///
/// Used by retry logic, compaction triggers, and error reporting to distinguish
/// transient from permanent failures without relying on string pattern matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmErrorKind {
    /// Model input exceeds the context window limit.
    ContextOverflow,
    /// Provider rate-limited the request. `retry_after_secs` is a hint (0 = unknown).
    RateLimit { retry_after_secs: u64 },
    /// Authentication credentials are missing or invalid.
    AuthFailed,
    /// The request itself is malformed or uses an unsupported feature.
    InvalidRequest,
    /// The provider service is temporarily unavailable or overloaded.
    ServiceUnavailable,
    /// An unclassified error (may or may not be retryable based on context).
    Unknown,
}

impl LlmErrorKind {
    /// Returns `true` for errors that are worth retrying (transient failures).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmErrorKind::RateLimit { .. } | LlmErrorKind::ServiceUnavailable
        )
    }

    /// Returns `true` when the context window has been exceeded (trigger compaction).
    #[must_use]
    pub fn is_context_overflow(&self) -> bool {
        matches!(self, LlmErrorKind::ContextOverflow)
    }
}

/// Classify an HTTP error response into an [`LlmErrorKind`].
///
/// Inspects the HTTP status code and optionally the response body to determine
/// the semantic error category.
///
/// Used by all provider clients so that the retry and compaction logic can
/// operate on typed errors rather than string patterns.
#[must_use]
pub fn classify_http_error(status: u16, body: &str) -> LlmErrorKind {
    let body_lower = body.to_ascii_lowercase();

    match status {
        // Context overflow — various providers signal this as 400 with specific messages
        400 if body_lower.contains("context")
            && (body_lower.contains("too long")
                || body_lower.contains("overflow")
                || body_lower.contains("too large")
                || body_lower.contains("maximum")) =>
        {
            LlmErrorKind::ContextOverflow
        }
        // Also catch "maximum context length" style messages from OpenAI-compat providers
        400 if body_lower.contains("maximum context length")
            || body_lower.contains("context_length_exceeded") =>
        {
            LlmErrorKind::ContextOverflow
        }
        400 => LlmErrorKind::InvalidRequest,
        401 | 403 => LlmErrorKind::AuthFailed,
        429 => {
            // Try to extract retry-after from the body (some providers include it)
            let retry_after_secs = extract_retry_after(&body_lower);
            LlmErrorKind::RateLimit { retry_after_secs }
        }
        // Anthropic overload code
        529 => LlmErrorKind::ServiceUnavailable,
        500 | 502 | 503 | 504 => {
            if body_lower.contains("overloaded") || body_lower.contains("rate_limit") {
                LlmErrorKind::RateLimit {
                    retry_after_secs: 0,
                }
            } else {
                LlmErrorKind::ServiceUnavailable
            }
        }
        _ => LlmErrorKind::Unknown,
    }
}

/// Extract a `retry_after` hint in seconds from a rate-limit response body.
fn extract_retry_after(body_lower: &str) -> u64 {
    // Common formats: "retry after 30s", "retry_after: 60", "wait 15 seconds"
    for pattern in &["retry after ", "retry_after: ", "wait "] {
        if let Some(pos) = body_lower.find(pattern) {
            let after = &body_lower[pos + pattern.len()..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(secs) = digits.parse::<u64>() {
                return secs;
            }
        }
    }
    0
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider returned invalid response: {0}")]
    Invalid(String),
    #[error("config error: {0}")]
    Config(String),
    /// Classified provider error with structured kind information.
    #[error("provider error ({kind:?}): {message}")]
    Classified {
        /// Structured error classification.
        kind: LlmErrorKind,
        /// Human-readable error message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_429_is_ratelimit() {
        let kind = classify_http_error(429, "too many requests");
        assert!(matches!(kind, LlmErrorKind::RateLimit { .. }));
        assert!(kind.is_retryable());
    }

    #[test]
    fn classify_401_is_auth_failed() {
        let kind = classify_http_error(401, "unauthorized");
        assert_eq!(kind, LlmErrorKind::AuthFailed);
        assert!(!kind.is_retryable());
    }

    #[test]
    fn classify_400_context_overflow() {
        let kind = classify_http_error(400, "context length is too long for this model");
        assert_eq!(kind, LlmErrorKind::ContextOverflow);
        assert!(kind.is_context_overflow());
        assert!(!kind.is_retryable());
    }

    #[test]
    fn classify_400_context_length_exceeded() {
        let kind = classify_http_error(
            400,
            "This model's maximum context length is 4096 tokens. context_length_exceeded",
        );
        assert_eq!(kind, LlmErrorKind::ContextOverflow);
    }

    #[test]
    fn classify_503_service_unavailable() {
        let kind = classify_http_error(503, "service unavailable");
        assert_eq!(kind, LlmErrorKind::ServiceUnavailable);
        assert!(kind.is_retryable());
    }

    #[test]
    fn classify_529_anthropic_overload() {
        let kind = classify_http_error(529, "overloaded");
        assert_eq!(kind, LlmErrorKind::ServiceUnavailable);
        assert!(kind.is_retryable());
    }

    #[test]
    fn classify_400_plain_invalid() {
        let kind = classify_http_error(400, "invalid request body");
        assert_eq!(kind, LlmErrorKind::InvalidRequest);
        assert!(!kind.is_retryable());
    }

    #[test]
    fn extract_retry_after_from_body() {
        let kind = classify_http_error(429, "rate limited, retry after 30s");
        assert!(matches!(
            kind,
            LlmErrorKind::RateLimit {
                retry_after_secs: 30
            }
        ));
    }

    #[test]
    fn extract_retry_after_ms_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after-ms", "1500".parse().unwrap());
        assert_eq!(extract_retry_after_header(&headers), Some(2)); // rounds up
    }

    #[test]
    fn extract_retry_after_secs_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "30".parse().unwrap());
        assert_eq!(extract_retry_after_header(&headers), Some(30));
    }

    #[test]
    fn extract_retry_after_ms_takes_priority() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after-ms", "5000".parse().unwrap());
        headers.insert("retry-after", "60".parse().unwrap());
        assert_eq!(extract_retry_after_header(&headers), Some(5));
    }

    #[test]
    fn extract_retry_after_header_missing() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(extract_retry_after_header(&headers), None);
    }

    #[test]
    fn classify_with_headers_overrides_retry_after() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "45".parse().unwrap());
        let kind = classify_http_error_with_headers(429, "too many requests", &headers);
        assert!(matches!(
            kind,
            LlmErrorKind::RateLimit {
                retry_after_secs: 45
            }
        ));
    }

    #[test]
    fn classified_error_display_contains_kind() {
        let err = LlmError::Classified {
            kind: LlmErrorKind::RateLimit {
                retry_after_secs: 60,
            },
            message: "provider returned 429: too many requests".to_string(),
        };
        let display = err.to_string();
        assert!(display.contains("RateLimit"));
        assert!(display.contains("60"));
    }
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::Invalid(
            "chat is not supported by the active provider client".to_string(),
        ))
    }
}

pub(crate) const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const HTTP_RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const HTTP_RESPONSE_BODY_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const HTTP_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn llm_http_client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|err| LlmError::Transport(format!("failed to build http client: {err}")))
}

/// Extract a `retry-after` hint from HTTP response headers.
///
/// Supports both `retry-after-ms` (milliseconds, priority) and `retry-after`
/// (seconds) headers. Returns seconds (rounded up for ms values).
///
/// # Errors
///
/// Returns `None` if neither header is present or parseable.
#[must_use]
pub(crate) fn extract_retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    if let Some(val) = headers.get("retry-after-ms").and_then(|v| v.to_str().ok()) {
        if let Ok(ms) = val.parse::<u64>() {
            return Some(ms.div_ceil(1000));
        }
    }
    if let Some(val) = headers.get("retry-after").and_then(|v| v.to_str().ok()) {
        if let Ok(secs) = val.parse::<u64>() {
            return Some(secs);
        }
    }
    None
}

/// Classify an HTTP error response, applying `retry-after` header hints.
///
/// Calls [`classify_http_error`] and overrides `retry_after_secs` in
/// [`LlmErrorKind::RateLimit`] when the HTTP headers provide a nonzero value.
#[must_use]
pub(crate) fn classify_http_error_with_headers(
    status: u16,
    body: &str,
    headers: &reqwest::header::HeaderMap,
) -> LlmErrorKind {
    let mut kind = classify_http_error(status, body);
    if let (Some(secs), LlmErrorKind::RateLimit { retry_after_secs }) =
        (extract_retry_after_header(headers), &mut kind)
    {
        if *retry_after_secs == 0 {
            *retry_after_secs = secs;
        }
    }
    kind
}

/// A [`LlmClient`] wrapper that allows hot-swapping the underlying client at runtime.
///
/// The TUI constructs the engine with a `SwappableLlmClient` and retains the
/// shared `Arc<RwLock<…>>` cell.  When the user switches models in the model
/// picker the TUI calls [`build_client`](crate::build_client) with the new
/// config and writes the new client into the cell — no engine restart required.
pub struct SwappableLlmClient {
    inner: Arc<std::sync::RwLock<Arc<dyn LlmClient>>>,
}

impl SwappableLlmClient {
    /// Wrap `initial` in a `SwappableLlmClient`.
    ///
    /// Returns the wrapper **and** the shared cell so the caller can swap the
    /// inner client later (e.g. after a model switch in the TUI).
    #[must_use]
    pub fn new(initial: Arc<dyn LlmClient>) -> (Self, Arc<std::sync::RwLock<Arc<dyn LlmClient>>>) {
        let cell = Arc::new(std::sync::RwLock::new(initial));
        (
            Self {
                inner: cell.clone(),
            },
            cell,
        )
    }
}

#[async_trait]
impl LlmClient for SwappableLlmClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let client = self
            .inner
            .read()
            .expect("llm client lock not poisoned")
            .clone();
        client.complete(request).await
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let client = self
            .inner
            .read()
            .expect("llm client lock not poisoned")
            .clone();
        client.chat(request).await
    }
}

#[derive(Debug, Default)]
pub struct NullLlmClient;

#[async_trait]
impl LlmClient for NullLlmClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: format!(
                "null-llm response (model={}): {}",
                request.model, request.prompt
            ),
            chunks: Vec::new(),
        })
    }
}
