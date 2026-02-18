// ── Session and agent status tracking ────────────────────────────────
//
// Tracks the lifecycle status of sessions and agent runs.
// Based on opencode's session/status.ts

use serde::{Deserialize, Serialize};

/// Status of a session or agent run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionStatus {
    /// Session is idle (no active agent run).
    Idle,
    
    /// Session/agent is currently active and running.
    Busy,
    
    /// Agent is retrying after a transient error.
    Retry {
        attempt: u32,
        message: String,
        /// Unix timestamp (ms) when next retry will occur
        next_retry_at: i64,
    },
}

impl SessionStatus {
    /// Returns true if the status represents an active/running state.
    pub fn is_active(&self) -> bool {
        matches!(self, SessionStatus::Busy | SessionStatus::Retry { .. })
    }
    
    /// Returns true if the session is idle.
    pub fn is_idle(&self) -> bool {
        matches!(self, SessionStatus::Idle)
    }
    
    /// Returns a human-readable display string.
    pub fn as_str(&self) -> &str {
        match self {
            SessionStatus::Idle => "idle",
            SessionStatus::Busy => "busy",
            SessionStatus::Retry { .. } => "retry",
        }
    }
    
    /// Returns an emoji/icon representation.
    pub fn icon(&self) -> &'static str {
        match self {
            SessionStatus::Idle => "⏸️",
            SessionStatus::Busy => "🔄",
            SessionStatus::Retry { .. } => "⏳",
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Idle => write!(f, "idle"),
            SessionStatus::Busy => write!(f, "busy"),
            SessionStatus::Retry { attempt, .. } => write!(f, "retry (attempt {})", attempt),
        }
    }
}

impl Default for SessionStatus {
    fn default() -> Self {
        SessionStatus::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn status_is_active() {
        assert!(!SessionStatus::Idle.is_active());
        assert!(SessionStatus::Busy.is_active());
        assert!(SessionStatus::Retry {
            attempt: 1,
            message: "test".to_string(),
            next_retry_at: 0,
        }
        .is_active());
    }
    
    #[test]
    fn status_is_idle() {
        assert!(SessionStatus::Idle.is_idle());
        assert!(!SessionStatus::Busy.is_idle());
        assert!(!SessionStatus::Retry {
            attempt: 1,
            message: "test".to_string(),
            next_retry_at: 0,
        }
        .is_idle());
    }
    
    #[test]
    fn status_display() {
        assert_eq!(SessionStatus::Idle.to_string(), "idle");
        assert_eq!(SessionStatus::Busy.to_string(), "busy");
        assert_eq!(
            SessionStatus::Retry {
                attempt: 2,
                message: "rate limit".to_string(),
                next_retry_at: 1000,
            }
            .to_string(),
            "retry (attempt 2)"
        );
    }
    
    #[test]
    fn status_icons() {
        assert_eq!(SessionStatus::Idle.icon(), "⏸️");
        assert_eq!(SessionStatus::Busy.icon(), "🔄");
        assert_eq!(
            SessionStatus::Retry {
                attempt: 1,
                message: "test".to_string(),
                next_retry_at: 0,
            }
            .icon(),
            "⏳"
        );
    }
    
    #[test]
    fn status_serialization() {
        let status = SessionStatus::Busy;
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"type\":\"busy\""));
        
        let deserialized: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, SessionStatus::Busy);
    }
    
    #[test]
    fn status_retry_serialization() {
        let status = SessionStatus::Retry {
            attempt: 2,
            message: "rate limit exceeded".to_string(),
            next_retry_at: 1234567890,
        };
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, status);
    }
}
