mod anthropic;
mod google;
mod openai;
mod usage_error;

// ── OpenAI / OpenAI-compatible ─────────────────────────────────────────────
pub(crate) use openai::{
    extract_openai_reasoning, extract_openai_stream_delta, extract_openai_stream_reasoning_delta,
    extract_openai_text, extract_openai_tool_calls, openai_message_value, openai_tool_spec_value,
};

// ── Anthropic ──────────────────────────────────────────────────────────────
pub(crate) use anthropic::{
    anthropic_messages_from_chat, extract_anthropic_reasoning, extract_anthropic_stream_delta,
    extract_anthropic_text, extract_anthropic_tool_calls,
};

#[cfg(test)]
pub(crate) use anthropic::extract_anthropic_stream_reasoning_delta;

// ── Google ─────────────────────────────────────────────────────────────────
#[allow(unused_imports)]
pub(crate) use google::sanitize_google_schema;
pub(crate) use google::{
    extract_google_reasoning, extract_google_text, extract_google_text_delta,
    extract_google_tool_calls, google_contents_from_chat, google_tools_from_specs,
};

// ── Gateway + usage + error ────────────────────────────────────────────────
#[allow(unused_imports)]
pub(crate) use usage_error::{error_http_status, extract_error_message, is_content_filter_error};
pub(crate) use usage_error::{
    extract_anthropic_usage, extract_gateway_stream_delta, extract_gateway_text,
    extract_google_usage, extract_openai_usage,
};

// ── Shared utility (used by openai and anthropic submodules) ───────────────

pub(crate) fn append_reasoning_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => out.push_str(text),
        serde_json::Value::Array(items) => {
            for item in items {
                append_reasoning_text(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["text", "thinking", "reasoning_text", "content", "summary"] {
                if let Some(value) = map.get(key) {
                    append_reasoning_text(value, out);
                }
            }
        }
        _ => {}
    }
}
