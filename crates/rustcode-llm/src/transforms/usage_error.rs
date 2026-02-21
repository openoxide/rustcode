use crate::types::TokenUsage;
use serde_json::Value;

// ── Gateway helpers ─────────────────────────────────────────────────────────

pub(crate) fn extract_gateway_text(value: &Value) -> Option<String> {
    if let Some(text) = super::openai::extract_openai_text(value) {
        return Some(text);
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    None
}

pub(crate) fn extract_gateway_stream_delta(value: &Value) -> Option<String> {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let candidates = ["textDelta", "delta", "text"];

    if kind.contains("delta") || kind.contains("text") {
        for key in candidates {
            if let Some(text) = value.get(key).and_then(Value::as_str) {
                return Some(text.to_string());
            }
        }
    }

    None
}

// ── Token usage extraction ──────────────────────────────────────────────────

/// Extract token usage from an OpenAI-compatible response.
/// Shape: `{ "usage": { "prompt_tokens": N, "completion_tokens": N, "total_tokens": N } }`
pub(crate) fn extract_openai_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let input = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input + output);
    let cache_read = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(TokenUsage {
        input,
        output,
        total,
        cache_read,
        cache_write: 0,
    })
}

/// Extract token usage from an Anthropic Messages API response.
/// Shape: `{ "usage": { "input_tokens": N, "output_tokens": N, "cache_read_input_tokens": N, "cache_creation_input_tokens": N } }`
pub(crate) fn extract_anthropic_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(TokenUsage {
        input,
        output,
        total: input + output,
        cache_read,
        cache_write,
    })
}

/// Extract token usage from a Google Generative AI response.
/// Shape: `{ "usageMetadata": { "promptTokenCount": N, "candidatesTokenCount": N, "totalTokenCount": N, "cachedContentTokenCount": N } }`
pub(crate) fn extract_google_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usageMetadata")?;
    let input = usage
        .get("promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("candidatesTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .get("totalTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(input + output);
    let cache_read = usage
        .get("cachedContentTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(TokenUsage {
        input,
        output,
        total,
        cache_read,
        cache_write: 0,
    })
}

// ── Error extraction ────────────────────────────────────────────────────────

/// Extract the human-readable error message from any provider's error response.
///
/// Handles the different error shapes used by `OpenAI`, Anthropic, Google, and
/// Vercel Gateway.
#[allow(dead_code)]
pub(crate) fn extract_error_message(value: &Value) -> Option<String> {
    // OpenAI / OpenRouter: { "error": { "message": "...", "type": "..." } }
    if let Some(error) = value.get("error") {
        if let Some(msg) = error.get("message").and_then(Value::as_str) {
            return Some(msg.to_string());
        }
        if let Some(msg) = error.as_str() {
            return Some(msg.to_string());
        }
    }

    // Anthropic: { "error": { "message": "..." }, "type": "error" }
    if value.get("type").and_then(Value::as_str) == Some("error") {
        if let Some(error) = value.get("error") {
            if let Some(msg) = error.get("message").and_then(Value::as_str) {
                return Some(msg.to_string());
            }
        }
    }

    // Google: { "error": { "message": "...", "status": "..." } }
    // (same shape as OpenAI — already handled above)

    // Fallback: top-level "message" field
    if let Some(msg) = value.get("message").and_then(Value::as_str) {
        return Some(msg.to_string());
    }

    None
}

/// Check if the error response indicates a content policy / safety filter violation.
#[allow(dead_code)]
pub(crate) fn is_content_filter_error(value: &Value) -> bool {
    // OpenAI: error.code == "content_filter" or error.code == "content_policy_violation"
    if let Some(code) = value
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
    {
        if code.contains("content_filter") || code.contains("content_policy") {
            return true;
        }
    }

    // Anthropic: error.type == "content_filter" or stop_reason == "end_turn" with
    // type == "error"
    if let Some(err_type) = value
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str)
    {
        if err_type.contains("content") {
            return true;
        }
    }

    // Google: candidates[].finishReason == "SAFETY"
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        for candidate in candidates {
            if candidate.get("finishReason").and_then(Value::as_str) == Some("SAFETY") {
                return true;
            }
        }
    }

    // Generic: check message text
    if let Some(msg) = extract_error_message(value) {
        let lower = msg.to_lowercase();
        if lower.contains("content policy")
            || lower.contains("content_filter")
            || lower.contains("safety")
        {
            return true;
        }
    }

    false
}

/// Extract the HTTP status code from a provider error response, if present.
#[allow(dead_code)]
pub(crate) fn error_http_status(value: &Value) -> Option<u16> {
    // { "error": { "status": 429 } } or { "status": 429 }
    value
        .get("error")
        .and_then(|e| e.get("status"))
        .and_then(Value::as_u64)
        .or_else(|| value.get("status").and_then(Value::as_u64))
        .and_then(|s| u16::try_from(s).ok())
}
