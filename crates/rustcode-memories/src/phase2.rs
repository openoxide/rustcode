// ── Phase 2: Memory Consolidation ────────────────────────────────────
//
// Reads all raw memory files, calls the LLM to synthesize them into a
// consolidated summary, and persists the result to summary.md.
//
// This phase runs on a background scheduler (every 10 minutes) rather
// than synchronously during a session.

use std::sync::Arc;

use thiserror::Error;
use tracing::{debug, info};

use rustcode_llm::{LlmClient, LlmRequest};

use crate::{
    model::MemorySummary,
    prompts::consolidation_prompt,
    storage::{MemoryStorage, StorageError},
};

/// Errors from Phase 2 consolidation.
#[derive(Debug, Error)]
pub enum ConsolidationError {
    #[error("no raw memories to consolidate")]
    NoRawMemories,

    #[error("LLM returned no content")]
    EmptyResponse,

    #[error("LLM error during consolidation: {0}")]
    Llm(String),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

/// Consolidate all raw memories into a single summary.
///
/// Skips if:
/// - No raw memories exist
/// - The existing summary is newer than all raw memories (nothing changed)
///
/// Returns `Ok(())` on success or when consolidation was skipped.
pub async fn consolidate_memories(
    llm: Arc<dyn LlmClient>,
    storage: &MemoryStorage,
    model: &str,
) -> Result<(), ConsolidationError> {
    let raw_memories = storage.load_all_raw();
    if raw_memories.is_empty() {
        debug!("phase2: no raw memories, skipping consolidation");
        return Err(ConsolidationError::NoRawMemories);
    }

    // Skip if summary is up to date (newer than all raw memories)
    if let Some(summary) = storage.load_summary() {
        let latest_raw = raw_memories.iter().map(|m| m.created_at).max().unwrap_or(0);
        if summary.updated_at >= latest_raw {
            debug!("phase2: summary is up to date, skipping consolidation");
            return Ok(());
        }
    }

    debug!(
        "phase2: consolidating {} raw memory files",
        raw_memories.len()
    );

    let raw_contents: Vec<String> = raw_memories.iter().map(|m| m.content.clone()).collect();

    let prompt = consolidation_prompt(&raw_contents);

    let request = LlmRequest {
        model: model.to_string(),
        prompt,
    };

    let response = llm
        .complete(request)
        .await
        .map_err(|e| ConsolidationError::Llm(e.to_string()))?;

    let content = response.text.trim().to_string();

    if content.is_empty() {
        return Err(ConsolidationError::EmptyResponse);
    }

    let summary = MemorySummary::new(&content);
    storage.save_summary(&summary)?;

    info!(
        "phase2: consolidated {} raw memories into summary ({} bytes)",
        raw_memories.len(),
        summary.content.len()
    );
    Ok(())
}
