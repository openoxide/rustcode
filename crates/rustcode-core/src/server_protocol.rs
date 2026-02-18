use serde::{Deserialize, Serialize};

use crate::session::{SessionInfo, StoredMessage};

pub const SERVER_API_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    SERVER_API_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1ErrorResponse {
    pub error: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1RunRequest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub prompt: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1SessionCreateRequest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1SessionCreateResponse {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub session: SessionInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1SessionsListResponse {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub sessions_root: String,
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V1SessionShowResponse {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub session: SessionInfo,
    pub messages: Vec<StoredMessage>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;
    use crate::session::{MessageRole, StoredToolCall};

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/fixtures")
            .join(name)
    }

    fn read_fixture_json(name: &str) -> Value {
        let path = fixture_path(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
        serde_json::from_str::<Value>(&raw)
            .unwrap_or_else(|err| panic!("failed to parse fixture {}: {err}", path.display()))
    }

    #[test]
    fn v1_run_request_fixture_matches_struct_shape() {
        let fixture = read_fixture_json("server_v1_run_request.json");
        let sample = V1RunRequest {
            schema_version: 1,
            prompt: "hello".to_string(),
            session_id: Some("s-1".to_string()),
        };
        let sample_value = serde_json::to_value(sample).expect("to_value");
        assert_eq!(fixture, sample_value);
    }

    #[test]
    fn v1_session_create_request_fixture_matches_struct_shape() {
        let fixture = read_fixture_json("server_v1_session_create_request.json");
        let sample = V1SessionCreateRequest {
            schema_version: 1,
            title: Some("t1".to_string()),
        };
        let sample_value = serde_json::to_value(sample).expect("to_value");
        assert_eq!(fixture, sample_value);
    }

    #[test]
    fn v1_sessions_list_response_fixture_matches_struct_shape() {
        let fixture = read_fixture_json("server_v1_sessions_list_response.json");
        let sample = V1SessionsListResponse {
            schema_version: 1,
            sessions_root: "/tmp/rustcode/sessions".to_string(),
            sessions: vec![SessionInfo {
                id: "s-1".to_string(),
                title: Some("t1".to_string()),
                created_at_unix_ms: 1000,
                updated_at_unix_ms: 2000,
                parent_id: None,
                cwd: "/tmp".to_string(),
                workspace_root: "/tmp".to_string(),
                model: "null".to_string(),
            }],
        };
        let sample_value = serde_json::to_value(sample).expect("to_value");
        assert_eq!(fixture, sample_value);
    }

    #[test]
    fn v1_session_show_response_fixture_matches_struct_shape() {
        let fixture = read_fixture_json("server_v1_session_show_response.json");
        let sample = V1SessionShowResponse {
            schema_version: 1,
            session: SessionInfo {
                id: "s-1".to_string(),
                title: Some("t1".to_string()),
                created_at_unix_ms: 1000,
                updated_at_unix_ms: 2000,
                parent_id: None,
                cwd: "/tmp".to_string(),
                workspace_root: "/tmp".to_string(),
                model: "null".to_string(),
            },
            messages: vec![
                StoredMessage {
                    id: "m-1".to_string(),
                    role: MessageRole::User,
                    created_at_unix_ms: 1500,
                    content: Value::String("hello".to_string()),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::<StoredToolCall>::new(),
                },
                StoredMessage {
                    id: "m-2".to_string(),
                    role: MessageRole::Assistant,
                    created_at_unix_ms: 1600,
                    content: Value::String("hi".to_string()),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::<StoredToolCall>::new(),
                },
            ],
        };
        let sample_value = serde_json::to_value(sample).expect("to_value");
        assert_eq!(fixture, sample_value);
    }
}
