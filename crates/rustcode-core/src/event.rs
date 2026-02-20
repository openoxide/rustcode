use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub type EventId = u64;
pub const EVENT_SCHEMA_VERSION: u16 = 1;

fn default_schema_version() -> u16 {
    EVENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventScope {
    System,
    Command,
    Tool,
    Ui,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    CommandAccepted {
        name: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        output: String,
    },
    OutputChunk {
        text: String,
    },
    ServeRequest {
        method: String,
        path: String,
        status: u16,
    },
    Warning {
        message: String,
    },
    Failure {
        message: String,
    },
    /// Token usage from a single LLM step — accumulated by the TUI for display.
    UsageUpdate {
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
        cache_read: u64,
        cache_write: u64,
        context_limit: u64,
    },
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub id: EventId,
    pub timestamp: SystemTime,
    pub scope: EventScope,
    pub payload: EventPayload,
}

impl Event {
    #[must_use]
    pub fn new(id: EventId, scope: EventScope, payload: EventPayload) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            id,
            timestamp: SystemTime::now(),
            scope,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_event_without_schema_version_deserializes() {
        let raw = r#"{
            "id": 7,
            "timestamp": {"secs_since_epoch": 0, "nanos_since_epoch": 0},
            "scope": "System",
            "payload": {"type": "Completed"}
        }"#;

        let event: Event = serde_json::from_str(raw).expect("legacy event must deserialize");
        assert_eq!(event.schema_version, EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn new_events_set_schema_version() {
        let event = Event::new(1, EventScope::System, EventPayload::Completed);
        assert_eq!(event.schema_version, EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn serve_request_payload_round_trips() {
        let event = Event::new(
            2,
            EventScope::System,
            EventPayload::ServeRequest {
                method: "GET".to_string(),
                path: "/health".to_string(),
                status: 200,
            },
        );

        let encoded = serde_json::to_string(&event).expect("must serialize");
        let decoded: Event = serde_json::from_str(&encoded).expect("must deserialize");
        assert_eq!(decoded, event);
    }
}
