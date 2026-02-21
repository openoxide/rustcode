// ── Background task scheduler ─────────────────────────────────────────
//
// Runs periodic maintenance tasks in dedicated tokio tasks.
// Currently drives memory consolidation every 10 minutes.
//
// Caller owns the `SchedulerHandle` and must call `shutdown()` to
// cleanly abort all background tasks.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use rustcode_llm::LlmClient;
use rustcode_memories::phase2::consolidate_memories;
use rustcode_memories::storage::MemoryStorage;

/// Interval between memory consolidation runs.
const MEMORY_CONSOLIDATION_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Handle to all background scheduler tasks.
///
/// Drop or call [`SchedulerHandle::shutdown`] to stop all tasks.
pub struct SchedulerHandle {
    cancel: CancellationToken,
    handles: Vec<JoinHandle<()>>,
}

impl SchedulerHandle {
    /// Start background tasks.
    ///
    /// - If `memories` is `Some`, starts the memory consolidation task.
    #[must_use]
    pub fn start(memories: Option<(Arc<MemoryStorage>, Arc<dyn LlmClient>, String)>) -> Self {
        let cancel = CancellationToken::new();
        let mut handles = Vec::new();

        if let Some((storage, llm, model)) = memories {
            let cancel_clone = cancel.clone();
            handles.push(tokio::spawn(async move {
                memory_consolidation_loop(storage, llm, model, cancel_clone).await;
            }));
        }

        Self { cancel, handles }
    }

    /// Signal all background tasks to stop and await their completion.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        for handle in self.handles {
            // Best-effort: ignore errors if the task already finished
            let _ = handle.await;
        }
    }
}

impl Default for SchedulerHandle {
    fn default() -> Self {
        Self {
            cancel: CancellationToken::new(),
            handles: Vec::new(),
        }
    }
}

/// Background loop that periodically consolidates memories.
async fn memory_consolidation_loop(
    storage: Arc<MemoryStorage>,
    llm: Arc<dyn LlmClient>,
    model: String,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(MEMORY_CONSOLIDATION_INTERVAL);
    // Skip the immediate first tick; start after first interval
    interval.tick().await;

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                debug!("scheduler: memory consolidation loop cancelled");
                break;
            }
            _ = interval.tick() => {
                if storage.is_disabled() {
                    debug!("scheduler: memory consolidation skipped (disabled)");
                    continue;
                }
                debug!("scheduler: running memory consolidation");
                match consolidate_memories(llm.clone(), &storage, &model).await {
                    Ok(()) => debug!("scheduler: memory consolidation complete"),
                    Err(e) => warn!("scheduler: memory consolidation skipped or failed: {e}"),
                }
            }
        }
    }
}
