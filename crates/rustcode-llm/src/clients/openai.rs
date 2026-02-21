use crate::endpoints::{
    apply_provider_default_headers, normalize_copilot_chat_endpoint,
    normalize_openai_chat_endpoint, truncate_for_error,
};
use crate::provider::model_for_provider;
use crate::streaming::{extract_stream_error_message, read_sse_or_body, StreamedProviderBody};
use crate::transforms::{
    extract_openai_reasoning, extract_openai_stream_delta, extract_openai_stream_reasoning_delta,
    extract_openai_text, extract_openai_tool_calls, extract_openai_usage, openai_message_value,
    openai_tool_spec_value,
};
use crate::types::{
    ChatMessage, ChatRequest, ChatResponse, ChatRole, LlmClient, LlmError, LlmRequest, LlmResponse,
    RequestInitiator, TokenUsage, ToolCall, ToolSpec, HTTP_RESPONSE_BODY_TIMEOUT,
    HTTP_RESPONSE_HEADER_TIMEOUT, HTTP_STREAM_IDLE_TIMEOUT,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use rustcode_auth::{AuthStore, StoredCredential};
use serde_json::{json, Value};
use std::collections::HashSet;
use tokio::time::timeout;

#[derive(Debug)]
pub(crate) struct OpenAiCompatibleClient {
    provider_id: String,
    endpoint: String,
    api_key: Option<String>,
    chatgpt_account_id: Option<String>,
    use_codex_endpoint: bool,
    http: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub(crate) fn new(
        provider_id: String,
        base_url: String,
        api_key: Option<String>,
    ) -> Result<Self, LlmError> {
        let mut use_codex_endpoint = false;
        let chatgpt_account_id = if provider_id == "openai" {
            let store = AuthStore::open_default();
            match store.get("openai").ok().flatten() {
                Some(StoredCredential::OAuth { account_id, .. }) => {
                    use_codex_endpoint = true;
                    account_id.filter(|value| !value.trim().is_empty())
                }
                _ => None,
            }
        } else {
            None
        };
        let endpoint = if use_codex_endpoint {
            "https://chatgpt.com/backend-api/codex/responses".to_string()
        } else if matches!(
            provider_id.as_str(),
            "github-copilot" | "github-copilot-enterprise"
        ) {
            normalize_copilot_chat_endpoint(&base_url)
        } else {
            normalize_openai_chat_endpoint(&base_url)
        };

        Ok(Self {
            provider_id,
            endpoint,
            api_key,
            chatgpt_account_id,
            use_codex_endpoint,
            http: crate::types::llm_http_client()?,
        })
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let requested_model = model_for_provider(&self.provider_id, &request.model);
        let model =
            normalize_openai_model(&self.provider_id, self.use_codex_endpoint, &requested_model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }
        if let Some(account_id) = &self.chatgpt_account_id {
            let value = HeaderValue::from_str(account_id)
                .map_err(|err| LlmError::Config(format!("invalid ChatGPT-Account-Id: {err}")))?;
            headers.insert(HeaderName::from_static("chatgpt-account-id"), value);
        }
        apply_provider_default_headers(&self.provider_id, &mut headers, RequestInitiator::User)
            .map_err(LlmError::Config)?;

        let mut payload = if self.use_codex_endpoint {
            json!({
                "model": model,
                "input": [{
                    "role": "user",
                    "content": [{"type": "input_text", "text": request.prompt}],
                }],
                "stream": true,
                "store": false,
                "instructions": codex_default_instructions(),
            })
        } else {
            json!({
                "model": model,
                "messages": [{"role":"user", "content": request.prompt}],
                "stream": true,
                "store": false,
            })
        };
        if let Some(effort) = default_reasoning_effort(&self.provider_id, &model) {
            if self.use_codex_endpoint {
                payload["reasoning"] = json!({ "effort": effort });
            } else {
                payload["reasoning_effort"] = json!(effort);
            }
        }
        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&payload)
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
                .await
                .map_err(|_| {
                    LlmError::Transport("timed out reading provider error body".to_string())
                })?
                .map_err(|err| LlmError::Transport(err.to_string()))?;
            let kind = crate::types::classify_http_error(status.as_u16(), &body);
            return Err(LlmError::Classified {
                kind,
                message: format!(
                    "provider returned {}: {}",
                    status,
                    truncate_for_error(&body)
                ),
            });
        }

        match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
            StreamedProviderBody::Body(body) => {
                if self.use_codex_endpoint {
                    parse_codex_complete_body_json(&body)
                } else {
                    let parsed: Value = serde_json::from_str(&body).map_err(|err| {
                        LlmError::Invalid(format!("response is not valid JSON: {err}"))
                    })?;
                    let text = extract_openai_text(&parsed).ok_or_else(|| {
                        LlmError::Invalid(
                            "choices[0].message.content missing from provider response".to_string(),
                        )
                    })?;
                    Ok(LlmResponse {
                        text,
                        chunks: Vec::new(),
                    })
                }
            }
            StreamedProviderBody::SseEvents(events) => {
                if self.use_codex_endpoint {
                    return parse_codex_complete_sse_events(events);
                }
                let mut chunks = Vec::new();
                let mut combined = String::new();
                for event in events {
                    if event == "[DONE]" {
                        break;
                    }
                    let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(error_message) = extract_stream_error_message(&parsed) {
                        return Err(LlmError::Transport(format!(
                            "provider stream error: {error_message}"
                        )));
                    }
                    if let Some(delta) = extract_openai_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "openai-compatible stream did not include text deltas".to_string(),
                    ));
                }
                Ok(LlmResponse {
                    text: combined,
                    chunks,
                })
            }
        }
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let requested_model = model_for_provider(&self.provider_id, &request.model);
        let model =
            normalize_openai_model(&self.provider_id, self.use_codex_endpoint, &requested_model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }
        if let Some(account_id) = &self.chatgpt_account_id {
            let value = HeaderValue::from_str(account_id)
                .map_err(|err| LlmError::Config(format!("invalid ChatGPT-Account-Id: {err}")))?;
            headers.insert(HeaderName::from_static("chatgpt-account-id"), value);
        }
        apply_provider_default_headers(&self.provider_id, &mut headers, request.initiator)
            .map_err(LlmError::Config)?;

        let use_stream = self.use_codex_endpoint;

        let mut payload = if self.use_codex_endpoint {
            let input = codex_input_from_messages(&request.messages);
            let tools = request
                .tools
                .iter()
                .map(codex_tool_spec_value)
                .collect::<Vec<_>>();
            let mut payload = json!({
                "model": model,
                "input": input,
                "stream": true,
                "store": false,
                "instructions": codex_instructions_from_messages(&request.messages),
            });
            if !tools.is_empty() {
                payload["tools"] = Value::Array(tools);
                payload["tool_choice"] = json!("auto");
            }
            payload
        } else {
            let messages = request
                .messages
                .iter()
                .map(openai_message_value)
                .collect::<Vec<_>>();
            let tools = request
                .tools
                .iter()
                .map(openai_tool_spec_value)
                .collect::<Vec<_>>();
            json!({
                "model": model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
                "stream": false,
                "store": false,
            })
        };
        if let Some(effort) = default_reasoning_effort(&self.provider_id, &model) {
            if self.use_codex_endpoint {
                payload["reasoning"] = json!({ "effort": effort });
            } else {
                payload["reasoning_effort"] = json!(effort);
            }
        }
        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&payload)
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
                .await
                .map_err(|_| LlmError::Transport("timed out reading provider body".to_string()))?
                .map_err(|err| LlmError::Transport(err.to_string()))?;
            let kind = crate::types::classify_http_error(status.as_u16(), &body);
            return Err(LlmError::Classified {
                kind,
                message: format!(
                    "provider returned {}: {}",
                    status,
                    truncate_for_error(&body)
                ),
            });
        }

        if use_stream {
            return match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
                StreamedProviderBody::Body(body) => {
                    if self.use_codex_endpoint {
                        parse_codex_chat_body_json(&body)
                    } else {
                        parse_chat_body_json(&body)
                    }
                }
                StreamedProviderBody::SseEvents(events) => {
                    if self.use_codex_endpoint {
                        return parse_codex_chat_sse_events(events);
                    }
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
                        if let Some(error_message) = extract_stream_error_message(&parsed) {
                            return Err(LlmError::Transport(format!(
                                "provider stream error: {error_message}"
                            )));
                        }
                        if let Some(delta) = extract_openai_stream_delta(&parsed) {
                            text.push_str(&delta);
                        }
                        if let Some(delta) = extract_openai_stream_reasoning_delta(&parsed) {
                            if !delta.is_empty() {
                                reasoning_chunks.push(delta);
                            }
                        }
                        for call in extract_openai_tool_calls(&parsed) {
                            if seen_tool_call_ids.insert(call.id.clone()) {
                                tool_calls.push(call);
                            }
                        }
                        usage = extract_openai_usage(&parsed).or(usage);
                    }
                    if text.is_empty() && tool_calls.is_empty() {
                        return Err(LlmError::Invalid(
                            "openai-compatible stream did not include content or tool calls"
                                .to_string(),
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
            };
        }

        let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
            .await
            .map_err(|_| LlmError::Transport("timed out reading provider body".to_string()))?
            .map_err(|err| LlmError::Transport(err.to_string()))?;
        if self.use_codex_endpoint {
            parse_codex_chat_body_json(&body)
        } else {
            parse_chat_body_json(&body)
        }
    }
}

fn parse_codex_complete_body_json(body: &str) -> Result<LlmResponse, LlmError> {
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

fn parse_codex_complete_sse_events(events: Vec<String>) -> Result<LlmResponse, LlmError> {
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

fn parse_codex_chat_body_json(body: &str) -> Result<ChatResponse, LlmError> {
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

fn parse_codex_chat_sse_events(events: Vec<String>) -> Result<ChatResponse, LlmError> {
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

fn codex_input_from_messages(messages: &[ChatMessage]) -> Vec<Value> {
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

fn codex_tool_spec_value(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
    })
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

fn extract_codex_usage(value: &Value) -> Option<TokenUsage> {
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

fn extract_codex_error_message(value: &Value) -> Option<String> {
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

fn parse_chat_body_json(body: &str) -> Result<ChatResponse, LlmError> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
    // Check for error embedded in a 200 response (OpenRouter and some providers do this)
    if let Some(err_msg) = extract_stream_error_message(&parsed) {
        return Err(LlmError::Invalid(format!("provider error: {err_msg}")));
    }
    let tool_calls = extract_openai_tool_calls(&parsed);
    let text = extract_openai_text(&parsed).unwrap_or_default();
    let reasoning = extract_openai_reasoning(&parsed);
    if text.is_empty() && tool_calls.is_empty() {
        return Err(LlmError::Invalid(format!(
            "provider response did not include content or tool calls: {}",
            truncate_for_error(body)
        )));
    }

    let usage = extract_openai_usage(&parsed);

    Ok(ChatResponse {
        text,
        reasoning,
        reasoning_chunks: Vec::new(),
        tool_calls,
        usage,
    })
}

fn normalize_openai_model(provider_id: &str, use_codex_endpoint: bool, model: &str) -> String {
    if provider_id != "openai" {
        return model.to_string();
    }
    if use_codex_endpoint {
        return model.to_string();
    }

    // OpenAI's API endpoint can lag behind codex model aliases shown by other
    // clients. Fall back to the broadly available codex frontier default.
    if model.eq_ignore_ascii_case("gpt-5.3-codex") {
        return "gpt-5.2-codex".to_string();
    }
    model.to_string()
}

fn default_reasoning_effort(provider_id: &str, model: &str) -> Option<&'static str> {
    if provider_id != "openai" {
        return None;
    }
    if model.to_ascii_lowercase().starts_with("gpt-5") {
        return Some("high");
    }
    None
}

fn codex_default_instructions() -> &'static str {
    "You are Codex, a coding assistant running in a terminal."
}

fn codex_instructions_from_messages(messages: &[ChatMessage]) -> String {
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

#[cfg(test)]
mod tests {
    use super::{
        codex_default_instructions, codex_input_from_messages, codex_instructions_from_messages,
        codex_tool_spec_value, default_reasoning_effort, normalize_openai_model,
        parse_codex_chat_sse_events,
    };
    use crate::types::{ChatMessage, ChatRole, ToolCall, ToolSpec};
    use serde_json::json;

    #[test]
    fn openai_codex_53_falls_back_to_52() {
        assert_eq!(
            normalize_openai_model("openai", false, "gpt-5.3-codex"),
            "gpt-5.2-codex"
        );
    }

    #[test]
    fn openai_codex_endpoint_keeps_requested_model() {
        assert_eq!(
            normalize_openai_model("openai", true, "gpt-5.3-codex"),
            "gpt-5.3-codex"
        );
    }

    #[test]
    fn non_openai_models_are_not_rewritten() {
        assert_eq!(
            normalize_openai_model("openrouter", false, "gpt-5.3-codex"),
            "gpt-5.3-codex"
        );
    }

    #[test]
    fn openai_gpt5_defaults_to_high_effort() {
        assert_eq!(
            default_reasoning_effort("openai", "gpt-5.2-codex"),
            Some("high")
        );
    }

    #[test]
    fn non_openai_has_no_default_reasoning_effort() {
        assert_eq!(
            default_reasoning_effort("openrouter", "gpt-5.2-codex"),
            None
        );
    }

    #[test]
    fn codex_instructions_use_system_messages() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: json!("sys-1"),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::<ToolCall>::new(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: json!("hello"),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::<ToolCall>::new(),
            },
            ChatMessage {
                role: ChatRole::System,
                content: json!([{"type":"text","text":"sys-2"}]),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::<ToolCall>::new(),
            },
        ];

        assert_eq!(
            codex_instructions_from_messages(&messages),
            "sys-1\n\nsys-2".to_string()
        );
    }

    #[test]
    fn codex_instructions_fallback_to_default() {
        let messages = vec![ChatMessage {
            role: ChatRole::User,
            content: json!("hello"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::<ToolCall>::new(),
        }];
        assert_eq!(
            codex_instructions_from_messages(&messages),
            codex_default_instructions().to_string()
        );
    }

    #[test]
    fn codex_input_uses_responses_shape() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: json!("sys"),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: json!("hello"),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: json!("calling tool"),
                tool_call_id: None,
                tool_name: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"hello"}"#.to_string(),
                }],
            },
            ChatMessage {
                role: ChatRole::Tool,
                content: json!({"ok": true}),
                tool_call_id: Some("call_1".to_string()),
                tool_name: Some("echo".to_string()),
                tool_calls: Vec::new(),
            },
        ];

        let input = codex_input_from_messages(&messages);
        assert_eq!(
            input,
            vec![
                json!({
                    "role": "user",
                    "content": [{"type":"input_text","text":"hello"}]
                }),
                json!({
                    "role": "assistant",
                    "content": [{"type":"output_text","text":"calling tool"}]
                }),
                json!({
                    "type":"function_call",
                    "call_id":"call_1",
                    "name":"echo",
                    "arguments":"{\"text\":\"hello\"}"
                }),
                json!({
                    "type":"function_call_output",
                    "call_id":"call_1",
                    "output":"{\"ok\":true}"
                }),
            ]
        );
    }

    #[test]
    fn codex_tool_spec_uses_responses_function_format() {
        let spec = ToolSpec {
            name: "echo".to_string(),
            description: "Echo text".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        };
        let value = codex_tool_spec_value(&spec);
        assert_eq!(value.get("type"), Some(&json!("function")));
        assert_eq!(value.get("name"), Some(&json!("echo")));
        assert_eq!(value.get("description"), Some(&json!("Echo text")));
        assert!(value.get("function").is_none());
    }

    #[test]
    fn codex_stream_events_parse_text_tools_reasoning_and_usage() {
        let events = vec![
            json!({"type":"response.output_text.delta","delta":"hi"}).to_string(),
            json!({"type":"response.reasoning_summary_text.delta","delta":"think"}).to_string(),
            json!({
                "type":"response.output_item.done",
                "item":{"type":"function_call","call_id":"call_1","name":"echo","arguments":"{\"text\":\"hello\"}"}
            })
            .to_string(),
            json!({
                "type":"response.completed",
                "response":{"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":2}}}
            })
            .to_string(),
        ];

        let response =
            parse_codex_chat_sse_events(events).expect("codex stream should parse successfully");
        assert_eq!(response.text, "hi");
        assert_eq!(response.reasoning.as_deref(), Some("think"));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(response.tool_calls[0].name, "echo");
        assert_eq!(
            response.usage.expect("usage should exist").total,
            15,
            "usage should be parsed from response.completed"
        );
    }
}
