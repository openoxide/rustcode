use std::path::PathBuf;
use thiserror::Error;

use rustcode_core::session::SessionId;

#[cfg(test)]
mod tests;

mod defaults;
mod helpers;
mod prompt_history;
mod recorder;
mod session_store;

#[cfg(test)]
pub(crate) use helpers::now_unix_ms;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("session not found: {0}")]
    NotFound(SessionId),
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

pub const PROMPT_HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct PromptHistoryStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PromptHistoryFile {
    #[serde(default)]
    prompts: Vec<String>,
}

/// Append a prompt to history with shell-like behavior.
///
/// Returns `true` when a new entry was appended.
pub fn push_prompt_history_entry(history: &mut Vec<String>, prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    if history.last().is_some_and(|last| last == trimmed) {
        return false;
    }
    history.push(trimmed.to_string());
    while history.len() > PROMPT_HISTORY_LIMIT {
        history.remove(0);
    }
    true
}

#[derive(Debug, Clone)]
pub struct FileTranscriptRecorder {
    store: SessionStore,
}
