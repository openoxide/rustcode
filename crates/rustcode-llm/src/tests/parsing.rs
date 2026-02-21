use super::*;

#[test]
fn openai_endpoint_normalization_handles_v1_and_full_path() {
    assert_eq!(
        normalize_openai_chat_endpoint("https://api.openai.com/v1"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        normalize_openai_chat_endpoint("https://api.openai.com/v1/chat/completions"),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        normalize_openai_chat_endpoint("http://localhost:11434"),
        "http://localhost:11434/v1/chat/completions"
    );
}

#[test]
fn anthropic_endpoint_normalization_handles_v1_and_full_path() {
    assert_eq!(
        normalize_anthropic_messages_endpoint("https://api.anthropic.com"),
        "https://api.anthropic.com/v1/messages"
    );
    assert_eq!(
        normalize_anthropic_messages_endpoint("https://api.anthropic.com/v1"),
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn vercel_gateway_endpoint_normalization_appends_language_model_path() {
    assert_eq!(
        normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai"),
        "https://ai-gateway.vercel.sh/v1/ai/language-model"
    );
    assert_eq!(
        normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai/"),
        "https://ai-gateway.vercel.sh/v1/ai/language-model"
    );
    assert_eq!(
        normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai/language-model"),
        "https://ai-gateway.vercel.sh/v1/ai/language-model"
    );
}

#[test]
fn gateway_stream_delta_extracts_text_delta() {
    let payload = json!({
        "type": "text-delta",
        "textDelta": "hello"
    });
    assert_eq!(
        extract_gateway_stream_delta(&payload).as_deref(),
        Some("hello")
    );
    let metadata = json!({
        "type": "response-metadata",
        "timestamp": "2026-02-17T00:00:00Z"
    });
    assert!(extract_gateway_stream_delta(&metadata).is_none());
}

#[test]
fn parses_openai_string_and_array_content() {
    let string_content = json!({
        "choices": [{"message": {"content": "hello"}}]
    });
    assert_eq!(
        extract_openai_text(&string_content).as_deref(),
        Some("hello")
    );

    let array_content = json!({
        "choices": [{"message": {"content": [{"type":"text","text":"he"},{"type":"text","text":"llo"}]}}]
    });
    assert_eq!(
        extract_openai_text(&array_content).as_deref(),
        Some("hello")
    );
}

#[test]
fn parses_anthropic_text_content() {
    let payload = json!({
        "content": [
            {"type":"text","text":"hello"},
            {"type":"thinking","text":"ignored"},
            {"type":"text","text":" world"}
        ]
    });

    assert_eq!(
        extract_anthropic_text(&payload).as_deref(),
        Some("hello world")
    );
}

#[test]
fn parses_anthropic_tool_use_blocks() {
    let payload = json!({
        "content": [
            {"type":"tool_use","id":"toolu_1","name":"read","input":{"path":"README.md"}},
            {"type":"text","text":"ok"}
        ]
    });

    let calls = extract_anthropic_tool_calls(&payload);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "toolu_1");
    assert_eq!(calls[0].name, "read");
    assert!(calls[0].arguments.contains("README.md"));
}

#[test]
fn google_endpoint_normalization_appends_v1beta() {
    assert_eq!(
        normalize_google_base_url("https://generativelanguage.googleapis.com"),
        "https://generativelanguage.googleapis.com/v1beta"
    );
    assert_eq!(
        normalize_google_base_url("https://generativelanguage.googleapis.com/v1beta"),
        "https://generativelanguage.googleapis.com/v1beta"
    );
    assert_eq!(
        normalize_google_generate_endpoint("https://generativelanguage.googleapis.com"),
        "https://generativelanguage.googleapis.com/v1beta/models/*:generateContent"
    );
}

#[test]
fn parses_google_text_content() {
    let payload = json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"text":"hello"},
                    {"text":" world"}
                ]
            }
        }]
    });
    assert_eq!(
        extract_google_text(&payload).as_deref(),
        Some("hello world")
    );
}

#[test]
fn parses_google_tool_calls_from_response() {
    let payload = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "read",
                        "args": {"path":"README.md"}
                    }
                }]
            }
        }]
    });
    let calls = extract_google_tool_calls(&payload);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read");
    assert!(calls[0].arguments.contains("README.md"));
}

#[test]
fn google_schema_sanitizer_drops_additional_properties() {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" }
        },
        "definitions": {
            "nested": {
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "value": { "type": "string" }
                }
            }
        },
        "items": [
            { "type": "object", "additionalProperties": false }
        ]
    });

    let sanitized = sanitize_google_schema(&schema);
    assert!(sanitized.get("additionalProperties").is_none());
    assert!(sanitized
        .pointer("/definitions/nested/additionalProperties")
        .is_none());
    assert!(sanitized.pointer("/items/0/additionalProperties").is_none());
}

#[test]
fn parses_openai_stream_deltas() {
    let string_delta = json!({
        "choices": [{"delta": {"content": "hello"}}]
    });
    assert_eq!(
        extract_openai_stream_delta(&string_delta).as_deref(),
        Some("hello")
    );

    let array_delta = json!({
        "choices": [{"delta": {"content": [{"type":"text","text":"he"},{"type":"text","text":"llo"}]}}]
    });
    assert_eq!(
        extract_openai_stream_delta(&array_delta).as_deref(),
        Some("hello")
    );
}

#[test]
fn parses_openai_reasoning_from_content_parts() {
    let payload = json!({
        "choices": [{
            "message": {
                "content": [
                    {"type":"reasoning","text":"step 1. "},
                    {"type":"text","text":"final answer"}
                ]
            }
        }]
    });
    assert_eq!(
        extract_openai_reasoning(&payload).as_deref(),
        Some("step 1. ")
    );
    assert_eq!(
        extract_openai_text(&payload).as_deref(),
        Some("final answer")
    );
}

#[test]
fn parses_openai_stream_reasoning_delta() {
    let payload = json!({
        "choices": [{"delta": {"reasoning_text": "considering options..."}}]
    });
    assert_eq!(
        extract_openai_stream_reasoning_delta(&payload).as_deref(),
        Some("considering options...")
    );
}

#[test]
fn parses_openai_tool_calls_from_response() {
    let payload = json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });

    let calls = extract_openai_tool_calls(&payload);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "read");
    assert!(calls[0].arguments.contains("README.md"));
}

#[test]
fn parses_anthropic_stream_deltas() {
    let start = json!({
        "type": "content_block_start",
        "content_block": {"type": "text", "text": "hello"}
    });
    assert_eq!(
        extract_anthropic_stream_delta(&start).as_deref(),
        Some("hello")
    );

    let delta = json!({
        "type": "content_block_delta",
        "delta": {"type":"text_delta","text":" world"}
    });
    assert_eq!(
        extract_anthropic_stream_delta(&delta).as_deref(),
        Some(" world")
    );
}

#[test]
fn parses_anthropic_reasoning_blocks() {
    let payload = json!({
        "content": [
            {"type":"thinking","thinking":"internal step"},
            {"type":"text","text":"answer"}
        ]
    });
    assert_eq!(
        extract_anthropic_reasoning(&payload).as_deref(),
        Some("internal step")
    );
}

#[test]
fn parses_anthropic_stream_reasoning_deltas() {
    let payload = json!({
        "type": "content_block_delta",
        "delta": {"type":"thinking_delta","thinking":"step..."}
    });
    assert_eq!(
        extract_anthropic_stream_reasoning_delta(&payload).as_deref(),
        Some("step...")
    );
}

#[test]
fn parses_google_reasoning_parts() {
    let payload = json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"thought": true, "text":"internal"},
                    {"text":"answer"}
                ]
            }
        }]
    });
    assert_eq!(
        extract_google_reasoning(&payload).as_deref(),
        Some("internal")
    );
    assert_eq!(extract_google_text(&payload).as_deref(), Some("answer"));
}

#[test]
fn parses_sse_blocks_with_lf_and_crlf() {
    let mut lf = "data: one\n\ndata: two\n\n".to_string();
    let first = take_next_sse_block(&mut lf).expect("first block");
    let second = take_next_sse_block(&mut lf).expect("second block");
    assert_eq!(parse_sse_data_block(&first).as_deref(), Some("one"));
    assert_eq!(parse_sse_data_block(&second).as_deref(), Some("two"));

    let mut crlf = "data: a\r\n\r\ndata: b\r\n\r\n".to_string();
    let first = take_next_sse_block(&mut crlf).expect("first block");
    let second = take_next_sse_block(&mut crlf).expect("second block");
    assert_eq!(parse_sse_data_block(&first).as_deref(), Some("a"));
    assert_eq!(parse_sse_data_block(&second).as_deref(), Some("b"));
}

#[test]
fn extracts_stream_error_message_from_object_payload() {
    let payload = json!({
        "error": {"message": "insufficient_quota"}
    });
    assert_eq!(
        extract_stream_error_message(&payload).as_deref(),
        Some("insufficient_quota")
    );
}

// ── Token usage extraction tests ───────────────────────────────────────

#[test]
fn extracts_openai_usage_with_cache() {
    let payload = json!({
        "choices": [{"message": {"content": "hi"}}],
        "usage": {
            "prompt_tokens": 1500,
            "completion_tokens": 200,
            "total_tokens": 1700,
            "prompt_tokens_details": { "cached_tokens": 500 }
        }
    });
    let usage = extract_openai_usage(&payload).expect("usage should be present");
    assert_eq!(usage.input, 1500);
    assert_eq!(usage.output, 200);
    assert_eq!(usage.total, 1700);
    assert_eq!(usage.cache_read, 500);
    assert_eq!(usage.cache_write, 0);
}

#[test]
fn extracts_openai_usage_without_cache() {
    let payload = json!({
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
    });
    let usage = extract_openai_usage(&payload).expect("usage should be present");
    assert_eq!(usage.input, 100);
    assert_eq!(usage.output, 50);
    assert_eq!(usage.total, 150);
    assert_eq!(usage.cache_read, 0);
}

#[test]
fn extracts_anthropic_usage_with_cache() {
    let payload = json!({
        "content": [{"type": "text", "text": "hello"}],
        "usage": {
            "input_tokens": 2000,
            "output_tokens": 300,
            "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 100
        }
    });
    let usage = extract_anthropic_usage(&payload).expect("usage should be present");
    assert_eq!(usage.input, 2000);
    assert_eq!(usage.output, 300);
    assert_eq!(usage.total, 2300);
    assert_eq!(usage.cache_read, 800);
    assert_eq!(usage.cache_write, 100);
}

#[test]
fn extracts_google_usage_with_cached_content() {
    let payload = json!({
        "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
        "usageMetadata": {
            "promptTokenCount": 500,
            "candidatesTokenCount": 100,
            "totalTokenCount": 600,
            "cachedContentTokenCount": 200
        }
    });
    let usage = extract_google_usage(&payload).expect("usage should be present");
    assert_eq!(usage.input, 500);
    assert_eq!(usage.output, 100);
    assert_eq!(usage.total, 600);
    assert_eq!(usage.cache_read, 200);
    assert_eq!(usage.cache_write, 0);
}

#[test]
fn openai_usage_returns_none_when_missing() {
    let payload = json!({"choices": [{"message": {"content": "hi"}}]});
    assert!(extract_openai_usage(&payload).is_none());
}

#[test]
fn anthropic_usage_returns_none_when_missing() {
    let payload = json!({"content": [{"type": "text", "text": "hi"}]});
    assert!(extract_anthropic_usage(&payload).is_none());
}

#[test]
fn google_usage_returns_none_when_missing() {
    let payload = json!({"candidates": [{"content": {"parts": [{"text": "hi"}]}}]});
    assert!(extract_google_usage(&payload).is_none());
}
