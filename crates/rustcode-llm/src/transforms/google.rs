use crate::types::{ChatRequest, ChatRole, LlmError, ToolCall, ToolSpec};
use serde_json::{json, Value};

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
