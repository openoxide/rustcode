use crate::endpoints::truncate_for_error;
use crate::streaming::extract_stream_error_message;
use crate::types::{
    ChatMessage, ChatResponse, ChatRole, LlmError, LlmResponse, TokenUsage, ToolCall, ToolSpec,
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn parse_codex_complete_body_json(body: &str) -> Result<LlmResponse, LlmError> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
    if let Some(err_msg) = extract_codex_error_message(&parsed) {
        return Err(LlmError::Invalid(format!("provider error: {err_msg}")));
    }
    let text = extract_codex_response_text(&parsed).ok_or_else(|| {
        LlmError::Invalid("codex response did not include output text".to_string())
    })?;
    Ok(LlmResponse {
        text,
        chunks: Vec::new(),
    })
}

pub(super) fn parse_codex_complete_sse_events(
    events: Vec<String>,
) -> Result<LlmResponse, LlmError> {
    let mut chunks = Vec::new();
    let mut combined = String::new();
    for event in events {
        if event == "[DONE]" {
            break;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
            continue;
        };
        if let Some(error_message) = extract_codex_error_message(&parsed) {
            return Err(LlmError::Transport(format!(
                "provider stream error: {error_message}"
            )));
        }
        if let Some(delta) = extract_codex_stream_text_delta(&parsed) {
            if !delta.is_empty() {
                combined.push_str(&delta);
                chunks.push(delta);
            }
        } else if combined.is_empty() {
            if let Some(done_text) = extract_codex_stream_done_message_text(&parsed) {
                combined.push_str(&done_text);
                chunks.push(done_text);
            }
        }
    }
    if combined.is_empty() {
        return Err(LlmError::Invalid(
            "codex stream did not include text deltas".to_string(),
        ));
    }
    Ok(LlmResponse {
        text: combined,
        chunks,
    })
}

pub(super) fn parse_codex_chat_body_json(body: &str) -> Result<ChatResponse, LlmError> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
    if let Some(err_msg) = extract_codex_error_message(&parsed) {
        return Err(LlmError::Invalid(format!("provider error: {err_msg}")));
    }

    let text = extract_codex_response_text(&parsed).unwrap_or_default();
    let reasoning = extract_codex_response_reasoning(&parsed);
    let tool_calls = extract_codex_response_tool_calls(&parsed);
    if text.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::Invalid(format!(
            "codex response did not include content or tool calls: {}",
            truncate_for_error(body)
        )));
    }

    Ok(ChatResponse {
        text,
        reasoning,
        reasoning_chunks: Vec::new(),
        tool_calls,
        usage: extract_codex_usage(&parsed),
    })
}

pub(super) fn parse_codex_chat_sse_events(events: Vec<String>) -> Result<ChatResponse, LlmError> {
    let mut text = String::new();
    let mut reasoning_chunks = Vec::new();
    let mut tool_calls = Vec::new();
    let mut seen_tool_call_ids = HashSet::new();
    let mut usage = None;
    for event in events {
        if event == "[DONE]" {
            break;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
            continue;
        };
        if let Some(error_message) = extract_codex_error_message(&parsed) {
            return Err(LlmError::Transport(format!(
                "provider stream error: {error_message}"
            )));
        }
        if let Some(delta) = extract_codex_stream_text_delta(&parsed) {
            text.push_str(&delta);
        } else if text.is_empty() {
            if let Some(done_text) = extract_codex_stream_done_message_text(&parsed) {
                text.push_str(&done_text);
            }
        }
        if let Some(delta) = extract_codex_stream_reasoning_delta(&parsed) {
            if !delta.is_empty() {
                reasoning_chunks.push(delta);
            }
        } else if reasoning_chunks.is_empty() {
            if let Some(done_reasoning) = extract_codex_stream_done_reasoning_text(&parsed) {
                if !done_reasoning.is_empty() {
                    reasoning_chunks.push(done_reasoning);
                }
            }
        }
        for call in extract_codex_stream_tool_calls(&parsed) {
            if seen_tool_call_ids.insert(call.id.clone()) {
                tool_calls.push(call);
            }
        }
        usage = extract_codex_usage(&parsed).or(usage);
    }
    if text.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::Invalid(
            "codex stream did not include content or tool calls".to_string(),
        ));
    }
    Ok(ChatResponse {
        text,
        reasoning: if reasoning_chunks.is_empty() {
            None
        } else {
            Some(reasoning_chunks.join(""))
        },
        reasoning_chunks,
        tool_calls,
        usage,
    })
}

pub(super) fn codex_input_from_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        match message.role {
            ChatRole::System => {}
            ChatRole::User => {
                let content = codex_text_content_parts(&message.content, "input_text");
                if !content.is_empty() {
                    input.push(json!({
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            ChatRole::Assistant => {
                let content = codex_text_content_parts(&message.content, "output_text");
                if !content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": content,
                    }));
                }
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
            }
            ChatRole::Tool => {
                let Some(call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": codex_tool_output_string(&message.content),
                }));
            }
        }
    }
    input
}

pub(super) fn codex_tool_spec_value(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
    })
}

pub(super) fn codex_default_instructions() -> &'static str {
    "You are Codex, a coding assistant running in a terminal."
}

pub(super) fn codex_instructions_from_messages(messages: &[ChatMessage]) -> String {
    let mut system_chunks = Vec::new();
    for message in messages {
        if message.role != ChatRole::System {
            continue;
        }
        if let Some(text) = message_content_text(&message.content) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                system_chunks.push(trimmed.to_string());
            }
        }
    }
    if system_chunks.is_empty() {
        codex_default_instructions().to_string()
    } else {
        system_chunks.join("\n\n")
    }
}

fn codex_text_content_parts(content: &Value, part_type: &str) -> Vec<Value> {
    if let Some(text) = content.as_str() {
        if text.trim().is_empty() {
            return Vec::new();
        }
        return vec![json!({
            "type": part_type,
            "text": text,
        })];
    }

    if let Some(parts) = content.as_array() {
        let mut result = Vec::new();
        for part in parts {
            let text = part
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str));
            if let Some(text) = text {
                if !text.trim().is_empty() {
                    result.push(json!({
                        "type": part_type,
                        "text": text,
                    }));
                }
            }
        }
        if !result.is_empty() {
            return result;
        }
    }

    if content.is_null() {
        return Vec::new();
    }

    let serialized = content.to_string();
    if serialized.trim().is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "type": part_type,
            "text": serialized,
        })]
    }
}

fn codex_tool_output_string(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        text.to_string()
    } else {
        content.to_string()
    }
}

fn extract_codex_response_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    let mut combined = String::new();
    for item in codex_output_items(value)? {
        append_codex_message_text_from_item(item, &mut combined);
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn extract_codex_response_reasoning(value: &Value) -> Option<String> {
    let mut combined = String::new();
    for item in codex_output_items(value)? {
        append_codex_reasoning_text_from_item(item, &mut combined);
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn extract_codex_response_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(items) = codex_output_items(value) else {
        return Vec::new();
    };
    let mut calls = Vec::new();
    for item in items {
        if let Some(call) = codex_function_call_from_item(item) {
            calls.push(call);
        }
    }
    calls
}

fn codex_output_items(value: &Value) -> Option<&Vec<Value>> {
    value.get("output").and_then(Value::as_array).or_else(|| {
        value
            .get("response")?
            .get("output")
            .and_then(Value::as_array)
    })
}

fn append_codex_message_text_from_item(item: &Value, out: &mut String) {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("output_text" | "input_text" | "text")
                    ) {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            out.push_str(text);
                        }
                    }
                }
            }
        }
        Some("output_text") => {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        _ => {}
    }
}

fn append_codex_reasoning_text_from_item(item: &Value, out: &mut String) {
    match item.get("type").and_then(Value::as_str) {
        Some("reasoning") => {
            if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                for part in summary {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        out.push_str(text);
                    }
                }
            }
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push_str(text);
            }
        }
        Some("message") => {
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("reasoning" | "reasoning_text")
                    ) {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            out.push_str(text);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn codex_function_call_from_item(item: &Value) -> Option<ToolCall> {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let id = item
        .get("call_id")
        .and_then(Value::as_str)
        .or_else(|| item.get("id").and_then(Value::as_str))?;
    let name = item.get("name").and_then(Value::as_str)?;
    let arguments = item
        .get("arguments")
        .map_or_else(String::new, |value| match value {
            Value::String(raw) => raw.clone(),
            other => other.to_string(),
        });
    Some(ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
    })
}

fn extract_codex_stream_text_delta(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("response.output_text.delta") {
        return value
            .get("delta")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    None
}

fn extract_codex_stream_reasoning_delta(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("response.reasoning_summary_text.delta") {
        return value
            .get("delta")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    None
}

fn extract_codex_stream_tool_calls(value: &Value) -> Vec<ToolCall> {
    if value.get("type").and_then(Value::as_str) != Some("response.output_item.done") {
        return Vec::new();
    }
    value
        .get("item")
        .and_then(codex_function_call_from_item)
        .into_iter()
        .collect()
}

fn extract_codex_stream_done_message_text(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("response.output_item.done") {
        return None;
    }
    let item = value.get("item")?;
    let mut combined = String::new();
    append_codex_message_text_from_item(item, &mut combined);
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn extract_codex_stream_done_reasoning_text(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("response.output_item.done") {
        return None;
    }
    let item = value.get("item")?;
    let mut combined = String::new();
    append_codex_reasoning_text_from_item(item, &mut combined);
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(super) fn extract_codex_usage(value: &Value) -> Option<TokenUsage> {
    value
        .get("usage")
        .and_then(codex_usage_from_value)
        .or_else(|| {
            value
                .get("response")?
                .get("usage")
                .and_then(codex_usage_from_value)
        })
}

fn codex_usage_from_value(value: &Value) -> Option<TokenUsage> {
    let input = value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = value
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut total = value
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = value
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = value
        .get("input_tokens_details")
        .and_then(|details| details.get("written_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if total == 0 && (input > 0 || output > 0) {
        total = input + output;
    }
    if input == 0 && output == 0 && total == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }
    Some(TokenUsage {
        input,
        output,
        total,
        cache_read,
        cache_write,
    })
}

pub(super) fn extract_codex_error_message(value: &Value) -> Option<String> {
    if let Some(err_msg) = extract_stream_error_message(value) {
        return Some(err_msg);
    }
    if value.get("type").and_then(Value::as_str) == Some("error") {
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return Some(message.to_string());
        }
        if let Some(detail) = value.get("detail").and_then(Value::as_str) {
            return Some(detail.to_string());
        }
        return Some(value.to_string());
    }
    value
        .get("detail")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn message_content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    if let Some(parts) = content.as_array() {
        let mut combined = String::new();
        for part in parts {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                combined.push_str(text);
            } else if let Some(text) = part
                .get("content")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                combined.push_str(text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }

    None
}
