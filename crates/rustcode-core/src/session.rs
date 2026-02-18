use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type SessionId = String;
pub type MessageId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMessage {
    pub id: MessageId,
    pub role: MessageRole,
    pub created_at_unix_ms: i64,

    /// Provider-neutral content. In practice this is usually a string, but some
    /// providers support structured content for multimodal.
    pub content: Value,

    /// Tool-call messages set `tool_call_id` so the next turn can correlate outputs.
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_calls: Vec<StoredToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: SessionId,
    pub title: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,

    pub parent_id: Option<SessionId>,

    /// The working directory used when the session was created.
    pub cwd: String,

    /// Workspace root used for tool path resolution.
    pub workspace_root: String,

    /// Model id as configured at runtime (e.g. "openai/gpt-4.1").
    pub model: String,
}
