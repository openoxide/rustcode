// ── Phase 1: Memory Extraction ────────────────────────────────────────
//
// Reads the last N messages from a completed session and calls the LLM
// to extract memorable facts as bullet points.  The result is saved as
// a raw memory file.

use std::sync::Arc;

use thiserror::Error;
use tracing::{debug, info};

use rustcode_llm::{LlmClient, LlmRequest};

use crate::{
    model::RawMemory,
    prompts::{extraction_prompt, EXTRACTION_PREAMBLE},
    storage::{MemoryStorage, StorageError},
};

/// Maximum number of messages to include in extraction (avoids token overflow).
const MAX_MESSAGES: usize = 50;

/// Errors from Phase 1 extraction.
#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("no messages to extract memories from")]
    NoMessages,

    #[error("LLM returned no content")]
    EmptyResponse,

    #[error("LLM error during extraction: {0}")]
    Llm(String),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

/// Extract memories from completed session messages and persist the result.
///
/// `messages` is a slice of `(role, content)` pairs from the session.
/// Roles other than `"user"` and `"assistant"` are ignored.
///
/// Returns `Ok(())` if extraction succeeded or was skipped (nothing worth remembering).
pub async fn extract_memories(
    session_id: &str,
    messages: &[(String, String)],
    llm: Arc<dyn LlmClient>,
    storage: &MemoryStorage,
    model: &str,
) -> Result<(), ExtractionError> {
    // Filter to only user/assistant messages, take the last MAX_MESSAGES
    let relevant: Vec<(String, String)> = messages
        .iter()
        .filter(|(role, _)| role == "user" || role == "assistant")
        .cloned()
        .collect();

    if relevant.is_empty() {
        debug!("phase1: no user/assistant messages in session {session_id}, skipping");
        return Err(ExtractionError::NoMessages);
    }

    let window: &[(String, String)] = if relevant.len() > MAX_MESSAGES {
        &relevant[relevant.len() - MAX_MESSAGES..]
    } else {
        &relevant
    };

    debug!(
        "phase1: extracting memories from {} messages for session {session_id}",
        window.len()
    );

    let prompt = extraction_prompt(window);

    let request = LlmRequest {
        model: model.to_string(),
        prompt,
    };

    let response = llm
        .complete(request)
        .await
        .map_err(|e| ExtractionError::Llm(e.to_string()))?;

    let content = response.text.trim().to_string();

    if content.is_empty() {
        return Err(ExtractionError::EmptyResponse);
    }

    // The model may respond with a sentinel if nothing was worth extracting
    let lower = content.to_ascii_lowercase();
    if lower.starts_with("no_memories") || lower.starts_with("no memories") {
        info!("phase1: model found no memories worth extracting for session {session_id}");
        return Ok(());
    }

    let _ = EXTRACTION_PREAMBLE; // used in prompts module
    let memory = RawMemory::new(session_id, &content);
    storage.save_raw(&memory)?;

    info!(
        "phase1: saved {} bytes of memories for session {session_id}",
        content.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_non_conversational_messages() {
        let messages = vec![
            ("system".to_string(), "You are rustcode".to_string()),
            ("user".to_string(), "hello".to_string()),
            ("tool".to_string(), "tool output".to_string()),
            ("assistant".to_string(), "hi there".to_string()),
        ];
        let relevant: Vec<_> = messages
            .iter()
            .filter(|(role, _)| role == "user" || role == "assistant")
            .collect();
        assert_eq!(relevant.len(), 2);
    }

    #[test]
    fn window_caps_at_max_messages() {
        let messages: Vec<(String, String)> = (0..100)
            .map(|i| {
                if i % 2 == 0 {
                    ("user".to_string(), format!("msg {i}"))
                } else {
                    ("assistant".to_string(), format!("resp {i}"))
                }
            })
            .collect();
        let capped: &[(String, String)] = if messages.len() > MAX_MESSAGES {
            &messages[messages.len() - MAX_MESSAGES..]
        } else {
            &messages
        };
        assert_eq!(capped.len(), MAX_MESSAGES);
    }
}
