use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub type EventId = u64;
pub const EVENT_SCHEMA_VERSION: u16 = 1;

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
    CommandAccepted { name: String },
    OutputChunk { text: String },
    Warning { message: String },
    Failure { message: String },
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub schema_version: u16,
    pub id: EventId,
    pub timestamp: SystemTime,
    pub scope: EventScope,
    pub payload: EventPayload,
}

impl Event {
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
