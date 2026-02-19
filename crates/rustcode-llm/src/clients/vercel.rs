use crate::endpoints::{
    apply_provider_default_headers, normalize_vercel_gateway_endpoint, truncate_for_error,
};
use crate::provider::model_for_provider;
use crate::streaming::{extract_stream_error_message, read_sse_or_body, StreamedProviderBody};
use crate::transforms::{extract_gateway_stream_delta, extract_gateway_text};
use crate::types::{
    LlmClient, LlmError, LlmRequest, LlmResponse, RequestInitiator, HTTP_RESPONSE_BODY_TIMEOUT,
    HTTP_RESPONSE_HEADER_TIMEOUT, HTTP_STREAM_IDLE_TIMEOUT,
};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::time::timeout;

#[derive(Debug)]
pub(crate) struct VercelAiGatewayClient {
    provider_id: String,
    endpoint: String,
    api_key: String,
    http: reqwest::Client,
}

impl VercelAiGatewayClient {
    pub(crate) fn new(
        provider_id: String,
        base_url: String,
        api_key: String,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            endpoint: normalize_vercel_gateway_endpoint(&base_url),
            api_key,
            http: crate::types::llm_http_client()?,
        })
    }
}

#[async_trait]
impl LlmClient for VercelAiGatewayClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);

        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {}", self.api_key);
        let value = HeaderValue::from_str(&bearer)
            .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
        headers.insert(AUTHORIZATION, value);
        headers.insert(
            HeaderName::from_static("ai-gateway-protocol-version"),
            HeaderValue::from_static("0.0.1"),
        );
        headers.insert(
            HeaderName::from_static("ai-gateway-auth-method"),
            HeaderValue::from_static("api-key"),
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-specification-version"),
            HeaderValue::from_static("2"),
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-id"),
            HeaderValue::from_str(&model)
                .map_err(|err| LlmError::Config(format!("invalid model header: {err}")))?,
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-streaming"),
            HeaderValue::from_static("true"),
        );
        apply_provider_default_headers("vercel", &mut headers, RequestInitiator::User)
            .map_err(LlmError::Config)?;

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "prompt": [{
                    "role": "user",
                    "content": [{"type": "text", "text": request.prompt}],
                }],
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
                let text = extract_gateway_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid("gateway response did not include text".to_string())
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
                    if let Some(delta) = extract_gateway_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "gateway stream did not include text deltas".to_string(),
                    ));
                }
                Ok(LlmResponse {
                    text: combined,
                    chunks,
                })
            }
        }
    }
}
