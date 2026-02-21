use crate::endpoints::{normalize_anthropic_messages_endpoint, truncate_for_error};
use crate::provider::model_for_provider;
use crate::streaming::{extract_stream_error_message, read_sse_or_body, StreamedProviderBody};
use crate::transforms::{
    anthropic_messages_from_chat, extract_anthropic_reasoning, extract_anthropic_stream_delta,
    extract_anthropic_text, extract_anthropic_tool_calls, extract_anthropic_usage,
};
use crate::types::{
    ChatRequest, ChatResponse, LlmClient, LlmError, LlmRequest, LlmResponse,
    HTTP_RESPONSE_BODY_TIMEOUT, HTTP_RESPONSE_HEADER_TIMEOUT, HTTP_STREAM_IDLE_TIMEOUT,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use tokio::time::timeout;

#[derive(Debug)]
pub(crate) struct AnthropicClient {
    provider_id: String,
    endpoint: String,
    api_key: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub(crate) fn new(
        provider_id: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            endpoint: normalize_anthropic_messages_endpoint(&base_url),
            api_key,
            http: crate::types::llm_http_client()?,
        })
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(&self.api_key)
                .map_err(|err| LlmError::Config(format!("invalid anthropic key header: {err}")))?,
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            HeaderName::from_static("anthropic-beta"),
            HeaderValue::from_static(
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
            ),
        );

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": request.prompt}],
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
                let parsed: Value = serde_json::from_str(&body).map_err(|err| {
                    LlmError::Invalid(format!("response is not valid JSON: {err}"))
                })?;
                let text = extract_anthropic_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid("content[0].text missing from anthropic response".to_string())
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
                    let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(error_message) = extract_stream_error_message(&parsed) {
                        return Err(LlmError::Transport(format!(
                            "provider stream error: {error_message}"
                        )));
                    }
                    if let Some(delta) = extract_anthropic_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "anthropic stream did not include text deltas".to_string(),
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
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(&self.api_key)
                .map_err(|err| LlmError::Config(format!("invalid anthropic key header: {err}")))?,
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            HeaderName::from_static("anthropic-beta"),
            HeaderValue::from_static(
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
            ),
        );

        let (system, messages) = anthropic_messages_from_chat(&request)?;
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "max_tokens": 1024,
                "system": system,
                "messages": messages,
                "tools": tools,
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

        let parsed: Value = serde_json::from_str(&body)
            .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
        // Check for error embedded in a 200 response (can happen with proxies)
        if let Some(err_msg) = extract_stream_error_message(&parsed) {
            return Err(LlmError::Invalid(format!("provider error: {err_msg}")));
        }
        let tool_calls = extract_anthropic_tool_calls(&parsed);
        let text = extract_anthropic_text(&parsed).unwrap_or_default();
        let reasoning = extract_anthropic_reasoning(&parsed);
        if text.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::Invalid(format!(
                "provider response did not include content or tool calls: {}",
                truncate_for_error(&body)
            )));
        }

        let usage = extract_anthropic_usage(&parsed);

        Ok(ChatResponse {
            text,
            reasoning,
            reasoning_chunks: Vec::new(),
            tool_calls,
            usage,
        })
    }
}
