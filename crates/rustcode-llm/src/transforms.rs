use crate::types::{ChatMessage, ChatRequest, ChatRole, LlmError, TokenUsage, ToolCall, ToolSpec};
use serde_json::{json, Value};

fn openai_role_value(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

fn openai_tool_call_value(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": call.arguments,
        }
    })
}

pub(crate) fn openai_message_value(message: &ChatMessage) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "role".to_string(),
        Value::String(openai_role_value(message.role).to_string()),
    );
    obj.insert("content".to_string(), message.content.clone());
    if message.role == ChatRole::Tool {
        if let Some(tool_call_id) = message.tool_call_id.as_ref() {
            obj.insert(
                "tool_call_id".to_string(),
                Value::String(tool_call_id.clone()),
            );
        }
        if let Some(tool_name) = message.tool_name.as_ref() {
            obj.insert("name".to_string(), Value::String(tool_name.clone()));
        }
    }
    if message.role == ChatRole::Assistant && !message.tool_calls.is_empty() {
        obj.insert(
            "tool_calls".to_string(),
            Value::Array(
                message
                    .tool_calls
                    .iter()
                    .map(openai_tool_call_value)
                    .collect(),
            ),
        );
    }
    Value::Object(obj)
}

pub(crate) fn openai_tool_spec_value(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

pub(crate) fn extract_openai_text(value: &Value) -> Option<String> {
    let message = value.get("choices")?.get(0)?;
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let content = message.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    let content_parts = content.as_array()?;
    let mut combined = String::new();
    for part in content_parts {
        let is_reasoning = matches!(
            part.get("type").and_then(Value::as_str),
            Some("reasoning" | "thinking")
        );
        if is_reasoning {
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_openai_reasoning(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;
    let mut combined = String::new();

    if let Some(message) = choice.get("message") {
        if let Some(text) = message.get("reasoning_text").and_then(Value::as_str) {
            combined.push_str(text);
        }
        if let Some(text) = message.get("reasoning_content").and_then(Value::as_str) {
            combined.push_str(text);
        }
        if let Some(reasoning) = message.get("reasoning") {
            append_reasoning_text(reasoning, &mut combined);
        }
        if let Some(details) = message.get("reasoning_details") {
            append_reasoning_text(details, &mut combined);
        }
        if let Some(parts) = message.get("content").and_then(Value::as_array) {
            for part in parts {
                let is_reasoning = matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("reasoning" | "thinking")
                );
                if !is_reasoning {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    combined.push_str(text);
                } else if let Some(text) = part.get("reasoning_text").and_then(Value::as_str) {
                    combined.push_str(text);
                } else if let Some(text) = part.get("thinking").and_then(Value::as_str) {
                    combined.push_str(text);
                }
            }
        }
    }

    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_openai_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(message) = value.get("choices").and_then(|choices| choices.get(0)) else {
        return Vec::new();
    };
    let Some(message) = message.get("message") else {
        return Vec::new();
    };
    let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut parsed = Vec::new();
    for call in tool_calls {
        let Some(id) = call.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(function) = call.get("function") else {
            continue;
        };
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(arguments) = function.get("arguments") else {
            continue;
        };
        let arguments = match arguments {
            Value::String(raw) => raw.clone(),
            other => other.to_string(),
        };
        parsed.push(ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        });
    }
    parsed
}

pub(crate) fn extract_openai_stream_delta(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;
    if let Some(text) = choice.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    let delta = choice.get("delta")?;
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    if let Some(parts) = delta.get("content").and_then(Value::as_array) {
        let mut combined = String::new();
        for part in parts {
            let is_reasoning = matches!(
                part.get("type").and_then(Value::as_str),
                Some("reasoning" | "thinking")
            );
            if is_reasoning {
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }

    None
}

pub(crate) fn extract_openai_stream_reasoning_delta(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;
    let delta = choice.get("delta")?;
    let mut combined = String::new();

    if let Some(text) = delta.get("reasoning_text").and_then(Value::as_str) {
        combined.push_str(text);
    }
    if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
        combined.push_str(text);
    }
    if let Some(reasoning) = delta.get("reasoning") {
        append_reasoning_text(reasoning, &mut combined);
    }
    if let Some(parts) = delta.get("content").and_then(Value::as_array) {
        for part in parts {
            let is_reasoning = matches!(
                part.get("type").and_then(Value::as_str),
                Some("reasoning" | "thinking")
            );
            if !is_reasoning {
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            } else if let Some(text) = part.get("reasoning_text").and_then(Value::as_str) {
                combined.push_str(text);
            } else if let Some(text) = part.get("thinking").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
    }

    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_anthropic_text(value: &Value) -> Option<String> {
    let content = value.get("content")?.as_array()?;
    let mut combined = String::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_anthropic_reasoning(value: &Value) -> Option<String> {
    let content = value.get("content")?.as_array()?;
    let mut combined = String::new();
    for item in content {
        let is_reasoning = matches!(
            item.get("type").and_then(Value::as_str),
            Some("thinking" | "reasoning")
        );
        if !is_reasoning {
            continue;
        }
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        } else if let Some(text) = item.get("thinking").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_anthropic_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(content) = value.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut calls = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let input = item.get("input").cloned().unwrap_or(Value::Null);
        calls.push(ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: input.to_string(),
        });
    }
    calls
}

pub(crate) fn extract_anthropic_stream_delta(value: &Value) -> Option<String> {
    let event_type = value.get("type").and_then(Value::as_str)?;
    match event_type {
        "content_block_delta" => value
            .get("delta")
            .filter(|delta| {
                matches!(
                    delta.get("type").and_then(Value::as_str),
                    Some("text_delta") | None
                )
            })
            .and_then(|delta| delta.get("text"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "content_block_start" => value
            .get("content_block")
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("text") | None
                )
            })
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn extract_anthropic_stream_reasoning_delta(value: &Value) -> Option<String> {
    let event_type = value.get("type").and_then(Value::as_str)?;
    match event_type {
        "content_block_delta" => value
            .get("delta")
            .filter(|delta| {
                matches!(
                    delta.get("type").and_then(Value::as_str),
                    Some("thinking_delta" | "reasoning_delta")
                )
            })
            .and_then(|delta| {
                delta
                    .get("text")
                    .or_else(|| delta.get("thinking"))
                    .and_then(Value::as_str)
            })
            .map(ToOwned::to_owned),
        "content_block_start" => value
            .get("content_block")
            .filter(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("thinking" | "reasoning")
                )
            })
            .and_then(|block| {
                block
                    .get("text")
                    .or_else(|| block.get("thinking"))
                    .and_then(Value::as_str)
            })
            .map(ToOwned::to_owned),
        _ => None,
    }
}

pub(crate) fn anthropic_messages_from_chat(
    request: &ChatRequest,
) -> Result<(String, Vec<Value>), LlmError> {
    let mut system = String::new();
    let mut messages: Vec<Value> = Vec::new();

    for message in &request.messages {
        match message.role {
            ChatRole::System => {
                if let Some(text) = message.content.as_str() {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(text);
                }
            }
            ChatRole::User => {
                let blocks = anthropic_content_blocks_from_value(&message.content);
                if !blocks.is_empty() {
                    messages.push(json!({"role":"user","content": blocks}));
                }
            }
            ChatRole::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message.content.as_str() {
                    if !text.is_empty() {
                        blocks.push(json!({"type":"text","text": text}));
                    }
                }
                for call in &message.tool_calls {
                    let input: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                if !blocks.is_empty() {
                    messages.push(json!({"role":"assistant","content": blocks}));
                }
            }
            ChatRole::Tool => {
                let Some(call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                let (content, is_error) = anthropic_tool_result_from_value(&message.content);
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                        "is_error": is_error
                    }]
                }));
            }
        }
    }

    Ok((system, messages))
}

fn anthropic_content_blocks_from_value(value: &Value) -> Vec<Value> {
    if let Some(text) = value.as_str() {
        if text.is_empty() {
            return Vec::new();
        }
        return vec![json!({"type":"text","text": text})];
    }
    if value.is_null() {
        return Vec::new();
    }
    vec![json!({"type":"text","text": value.to_string()})]
}

fn anthropic_tool_result_from_value(value: &Value) -> (String, bool) {
    let content = if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    };

    let is_error = match serde_json::from_str::<Value>(&content) {
        Ok(parsed) => parsed
            .get("ok")
            .and_then(Value::as_bool)
            .is_some_and(|ok| !ok),
        Err(_) => false,
    };

    (content, is_error)
}

pub(crate) fn extract_google_text(value: &Value) -> Option<String> {
    let candidates = value.get("candidates")?.as_array()?;
    let first = candidates.first()?;
    let content = first.get("content")?;
    let parts = content.get("parts")?.as_array()?;
    let mut combined = String::new();
    for part in parts {
        let is_reasoning = part.get("thought").and_then(Value::as_bool) == Some(true);
        if is_reasoning {
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

pub(crate) fn extract_google_text_delta(value: &Value) -> Option<String> {
    extract_google_text(value)
}

pub(crate) fn extract_google_reasoning(value: &Value) -> Option<String> {
    let candidates = value.get("candidates")?.as_array()?;
    let first = candidates.first()?;
    let content = first.get("content")?;
    let parts = content.get("parts")?.as_array()?;
    let mut combined = String::new();
    for part in parts {
        let is_reasoning = part.get("thought").and_then(Value::as_bool) == Some(true)
            || part.get("reasoning").and_then(Value::as_bool) == Some(true);
        if !is_reasoning {
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn append_reasoning_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => {
            for item in items {
                append_reasoning_text(item, out);
            }
        }
        Value::Object(map) => {
            for key in ["text", "thinking", "reasoning_text", "content", "summary"] {
                if let Some(value) = map.get(key) {
                    append_reasoning_text(value, out);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn extract_google_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(candidates) = value.get("candidates").and_then(Value::as_array) else {
        return Vec::new();
    };
    let Some(first) = candidates.first() else {
        return Vec::new();
    };
    let Some(parts) = first
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut calls = Vec::new();
    let mut idx = 1usize;
    for part in parts {
        let Some(call) = part.get("functionCall") else {
            continue;
        };
        let Some(name) = call.get("name").and_then(Value::as_str) else {
            continue;
        };
        let args = call.get("args").cloned().unwrap_or(Value::Null);
        calls.push(ToolCall {
            id: format!("call_{idx}"),
            name: name.to_string(),
            arguments: args.to_string(),
        });
        idx += 1;
    }
    calls
}

pub(crate) fn google_tools_from_specs(tools: &[ToolSpec]) -> Vec<Value> {
    if tools.is_empty() {
        return Vec::new();
    }
    let declarations = tools
        .iter()
        .map(|tool| {
            let parameters = sanitize_google_schema(&tool.parameters);
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": parameters,
            })
        })
        .collect::<Vec<_>>();
    vec![json!({
        "functionDeclarations": declarations
    })]
}

pub(crate) fn sanitize_google_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, child) in map {
                if key == "additionalProperties" {
                    continue;
                }
                sanitized.insert(key.clone(), sanitize_google_schema(child));
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_google_schema).collect()),
        _ => value.clone(),
    }
}

pub(crate) fn google_contents_from_chat(
    request: &ChatRequest,
) -> Result<(String, Vec<Value>), LlmError> {
    let mut system = String::new();
    let mut contents: Vec<Value> = Vec::new();

    for message in &request.messages {
        match message.role {
            ChatRole::System => {
                if let Some(text) = message.content.as_str() {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(text);
                }
            }
            ChatRole::User => {
                let text = google_text_from_value(&message.content);
                if !text.is_empty() {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{"text": text}],
                    }));
                }
            }
            ChatRole::Assistant => {
                let mut parts: Vec<Value> = Vec::new();
                let text = google_text_from_value(&message.content);
                if !text.is_empty() {
                    parts.push(json!({"text": text}));
                }
                for call in &message.tool_calls {
                    let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                    parts.push(json!({
                        "functionCall": {
                            "name": call.name,
                            "args": args,
                        }
                    }));
                }
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": "model",
                        "parts": parts,
                    }));
                }
            }
            ChatRole::Tool => {
                let Some(tool_name) = message.tool_name.as_deref() else {
                    return Err(LlmError::Invalid(
                        "tool result message missing tool_name for google provider".to_string(),
                    ));
                };
                let response = google_tool_response_object(&message.content);
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": tool_name,
                            "response": response,
                        }
                    }],
                }));
            }
        }
    }

    Ok((system, contents))
}

fn google_text_from_value(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if value.is_null() {
        return String::new();
    }
    value.to_string()
}

fn google_tool_response_object(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            if parsed.is_object() {
                return parsed;
            }
        }
        return json!({ "output": text });
    }
    if value.is_object() {
        return value.clone();
    }
    json!({ "output": value })
}

pub(crate) fn extract_gateway_text(value: &Value) -> Option<String> {
    if let Some(text) = extract_openai_text(value) {
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

// ── Token usage extraction ─────────────────────────────────────────────

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

// ── Error extraction ───────────────────────────────────────────────

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
