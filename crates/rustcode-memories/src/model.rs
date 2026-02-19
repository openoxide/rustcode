use std::time::{SystemTime, UNIX_EPOCH};

/// A raw memory extracted from a single completed session.
#[derive(Debug, Clone)]
pub struct RawMemory {
    /// ID of the session this memory was extracted from.
    pub session_id: String,
    /// Bullet-point facts extracted by the LLM.
    pub content: String,
    /// Unix timestamp (seconds) when this was extracted.
    pub created_at: u64,
}

impl RawMemory {
    /// Create a new `RawMemory` with the current timestamp.
    pub fn new(session_id: impl Into<String>, content: impl Into<String>) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            session_id: session_id.into(),
            content: content.into(),
            created_at,
        }
    }
}

/// A consolidated memory summary produced from all raw memories.
#[derive(Debug, Clone)]
pub struct MemorySummary {
    /// Markdown-formatted synthesis of all raw memories.
    pub content: String,
    /// Unix timestamp (seconds) when this summary was last updated.
    pub updated_at: u64,
}

impl MemorySummary {
    /// Create a new `MemorySummary` with the current timestamp.
    pub fn new(content: impl Into<String>) -> Self {
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            content: content.into(),
            updated_at,
        }
    }
}
