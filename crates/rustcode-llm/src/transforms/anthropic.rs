use crate::types::{ChatRequest, ChatRole, LlmError, ToolCall};
use serde_json::{json, Value};

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
