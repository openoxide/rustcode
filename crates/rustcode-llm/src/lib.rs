use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider returned invalid response: {0}")]
    Invalid(String),
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;
}

#[derive(Debug, Default)]
pub struct NullLlmClient;

#[async_trait]
impl LlmClient for NullLlmClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: format!(
                "null-llm response (model={}): {}",
                request.model, request.prompt
            ),
        })
    }
}
