use crate::endpoints::{
    apply_provider_default_headers, normalize_copilot_chat_endpoint,
    normalize_openai_chat_endpoint, truncate_for_error,
};
use crate::provider::model_for_provider;
use crate::streaming::{extract_stream_error_message, read_sse_or_body, StreamedProviderBody};
use crate::transforms::{
    extract_openai_stream_delta, extract_openai_text, extract_openai_tool_calls,
    extract_openai_usage, openai_message_value, openai_tool_spec_value,
};
use crate::types::{
    ChatRequest, ChatResponse, LlmClient, LlmError, LlmRequest, LlmResponse, RequestInitiator,
    HTTP_RESPONSE_BODY_TIMEOUT, HTTP_RESPONSE_HEADER_TIMEOUT, HTTP_STREAM_IDLE_TIMEOUT,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::time::timeout;

#[derive(Debug)]
pub(crate) struct OpenAiCompatibleClient {
    provider_id: String,
    endpoint: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub(crate) fn new(
        provider_id: String,
        base_url: String,
        api_key: Option<String>,
    ) -> Result<Self, LlmError> {
        let endpoint = if matches!(
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
            http: crate::types::llm_http_client()?,
        })
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }
        apply_provider_default_headers(&self.provider_id, &mut headers, RequestInitiator::User)
            .map_err(LlmError::Config)?;

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "messages": [{"role":"user", "content": request.prompt}],
                "stream": true,
            }))
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
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
            StreamedProviderBody::Body(body) => {
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
            StreamedProviderBody::SseEvents(events) => {
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
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }
        apply_provider_default_headers(&self.provider_id, &mut headers, request.initiator)
            .map_err(LlmError::Config)?;

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

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
                "stream": false,
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
            .await
            .map_err(|_| LlmError::Transport("timed out reading provider body".to_string()))?
            .map_err(|err| LlmError::Transport(err.to_string()))?;
        if !status.is_success() {
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        let parsed: Value = serde_json::from_str(&body)
            .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
        let tool_calls = extract_openai_tool_calls(&parsed);
        let text = extract_openai_text(&parsed).unwrap_or_default();
        if text.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::Invalid(
                "provider response did not include content or tool calls".to_string(),
            ));
        }

        let usage = extract_openai_usage(&parsed);

        Ok(ChatResponse {
            text,
            tool_calls,
            usage,
        })
    }
}
