use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Value,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub initiator: RequestInitiator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestInitiator {
    User,
    Agent,
}

/// Token usage statistics from an LLM API response.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    /// Tokens consumed by the input (prompt + messages).
    pub input: u64,
    /// Tokens generated in the output.
    pub output: u64,
    /// Total tokens (input + output). May differ from sum if provider reports differently.
    pub total: u64,
    /// Tokens read from cache (prompt caching).
    pub cache_read: u64,
    /// Tokens written to cache.
    pub cache_write: u64,
}

/// Response from an LLM chat completion.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// The assistant's text reply (may be empty if only tool calls).
    pub text: String,
    /// Tool calls requested by the assistant.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage statistics, if reported by the provider.
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub chunks: Vec<String>,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider returned invalid response: {0}")]
    Invalid(String),
    #[error("config error: {0}")]
    Config(String),
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::Invalid(
            "chat is not supported by the active provider client".to_string(),
        ))
    }
}

pub(crate) const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const HTTP_RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const HTTP_RESPONSE_BODY_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const HTTP_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn llm_http_client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .build()
        .map_err(|err| LlmError::Transport(format!("failed to build http client: {err}")))
}

#[derive(Debug, Default)]
pub struct NullLlmClient;

#[async_trait]
impl LlmClient for NullLlmClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: format!("null-llm response (model={}): {}", request.model, request.prompt),
            chunks: Vec::new(),
        })
    }
}
