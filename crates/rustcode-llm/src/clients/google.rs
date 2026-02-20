use crate::endpoints::{normalize_google_base_url, truncate_for_error};
use crate::provider::model_for_provider;
use crate::streaming::{extract_stream_error_message, read_sse_or_body, StreamedProviderBody};
use crate::transforms::{
    extract_google_text, extract_google_text_delta, extract_google_tool_calls,
    extract_google_usage, google_contents_from_chat, google_tools_from_specs,
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
pub(crate) struct GoogleGenerativeAiClient {
    provider_id: String,
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl GoogleGenerativeAiClient {
    pub(crate) fn new(
        provider_id: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            base_url,
            api_key,
            http: crate::types::llm_http_client()?,
        })
    }

    fn stream_endpoint(&self, model: &str) -> String {
        let base = normalize_google_base_url(&self.base_url);
        format!("{base}/models/{model}:streamGenerateContent?alt=sse")
    }

    fn generate_endpoint(&self, model: &str) -> String {
        let base = normalize_google_base_url(&self.base_url);
        format!("{base}/models/{model}:generateContent")
    }
}

#[async_trait]
impl LlmClient for GoogleGenerativeAiClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-goog-api-key"),
            HeaderValue::from_str(&self.api_key)
                .map_err(|err| LlmError::Config(format!("invalid google api key header: {err}")))?,
        );

        let response = self
            .http
            .post(self.stream_endpoint(&model))
            .headers(headers)
            .json(&json!({
                "contents": [{
                    "role": "user",
                    "parts": [{"text": request.prompt}],
                }],
                "generationConfig": {
                    "maxOutputTokens": 1024,
                }
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
                let text = extract_google_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid("google response did not include text".to_string())
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
                    if let Some(delta) = extract_google_text_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "google stream did not include text deltas".to_string(),
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
            HeaderName::from_static("x-goog-api-key"),
            HeaderValue::from_str(&self.api_key)
                .map_err(|err| LlmError::Config(format!("invalid google api key header: {err}")))?,
        );

        let (system, contents) = google_contents_from_chat(&request)?;
        let tools = google_tools_from_specs(&request.tools);

        let mut payload = serde_json::Map::new();
        payload.insert("contents".to_string(), Value::Array(contents));
        payload.insert(
            "generationConfig".to_string(),
            json!({
                "maxOutputTokens": 1024,
            }),
        );
        if !system.is_empty() {
            payload.insert(
                "systemInstruction".to_string(),
                json!({
                    "role": "system",
                    "parts": [{"text": system}],
                }),
            );
        }
        if !tools.is_empty() {
            payload.insert("tools".to_string(), Value::Array(tools));
        }

        let response = self
            .http
            .post(self.generate_endpoint(&model))
            .headers(headers)
            .json(&Value::Object(payload))
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
        // Check for error embedded in a 200 response
        if let Some(err_msg) = extract_stream_error_message(&parsed) {
            return Err(LlmError::Invalid(format!("provider error: {err_msg}")));
        }
        let tool_calls = extract_google_tool_calls(&parsed);
        let text = extract_google_text(&parsed).unwrap_or_default();
        if text.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::Invalid(format!(
                "provider response did not include content or tool calls: {}",
                truncate_for_error(&body)
            )));
        }
        let usage = extract_google_usage(&parsed);
        Ok(ChatResponse {
            text,
            tool_calls,
            usage,
        })
    }
}
