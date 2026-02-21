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
    ChatRequest, ChatResponse, LlmClient, LlmError, LlmRequest, LlmResponse, RequestInitiator,
    HTTP_RESPONSE_BODY_TIMEOUT, HTTP_RESPONSE_HEADER_TIMEOUT, HTTP_STREAM_IDLE_TIMEOUT,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use rustcode_auth::{AuthStore, StoredCredential};
use serde_json::{json, Value};
use std::collections::HashSet;
use tokio::time::timeout;

mod codex;
use self::codex::{
    codex_default_instructions, codex_input_from_messages, codex_instructions_from_messages,
    codex_tool_spec_value, parse_codex_chat_body_json, parse_codex_chat_sse_events,
    parse_codex_complete_body_json, parse_codex_complete_sse_events,
};

#[cfg(test)]
mod tests;

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
