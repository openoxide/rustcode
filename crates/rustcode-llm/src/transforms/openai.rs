use crate::types::{ChatMessage, ChatRole, ToolCall, ToolSpec};
use serde_json::{json, Value};

pub(super) fn openai_role_value(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

pub(super) fn openai_tool_call_value(call: &ToolCall) -> Value {
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
            super::append_reasoning_text(reasoning, &mut combined);
        }
        if let Some(details) = message.get("reasoning_details") {
            super::append_reasoning_text(details, &mut combined);
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
        super::append_reasoning_text(reasoning, &mut combined);
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
