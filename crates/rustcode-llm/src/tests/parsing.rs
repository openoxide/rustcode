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
