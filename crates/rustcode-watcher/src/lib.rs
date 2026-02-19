// ── rustcode-watcher ──────────────────────────────────────────────────────────
//
// Filesystem watcher for a workspace root.
//
// Uses the `notify` crate (platform native: kqueue on macOS, inotify on Linux)
// to watch a directory recursively and broadcast coarse-grained
// `FileChangedEvent`s to subscribers.
//
// Events are throttled/coalesced over a 500 ms window so that rapid bursts
// (e.g. a `cargo build` touching many files) produce a single broadcast.
//
// Reference: codex `crates/core/src/file_watcher.rs` (10 s throttle, skill roots).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use notify::{recommended_watcher, Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::broadcast;
use tokio::time::sleep;

/// Coalescing window — events are batched for this duration before broadcast.
const THROTTLE: Duration = Duration::from_millis(500);

/// Broadcast channel capacity.
const CHANNEL_CAPACITY: usize = 64;

// ── Public types ──────────────────────────────────────────────────────────────

/// An event from the file watcher.
#[derive(Debug, Clone)]
pub enum FileChangedEvent {
    /// One or more files in the watched workspace changed.
    WorkspaceChanged { paths: Vec<PathBuf> },
}

// ── Internal types ────────────────────────────────────────────────────────────

struct InnerWatcher {
    _watcher: Box<dyn Watcher + Send>,
}

// ── FileWatcher ───────────────────────────────────────────────────────────────

/// Watches a workspace root directory and broadcasts `FileChangedEvent`s.
///
/// Construct with [`FileWatcher::new`] or [`FileWatcher::noop`].
/// Use [`FileWatcher::subscribe`] to receive events.
pub struct FileWatcher {
    /// Underlying notify watcher — kept alive so watching continues (not read, just held).
    #[allow(dead_code)]
    inner: Option<Mutex<InnerWatcher>>,
    /// Broadcast sender; receivers are created via `subscribe()`.
    tx: broadcast::Sender<FileChangedEvent>,
}

impl FileWatcher {
    /// Start watching `workspace_root` recursively.
    ///
    /// Spawns a background tokio task that coalesces events and broadcasts
    /// them. Returns an error if the native watcher cannot be initialised
    /// (e.g. inotify limit reached).
    pub fn new(workspace_root: &Path) -> notify::Result<Self> {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        let tx_clone = tx.clone();

        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();

        let mut watcher = recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if should_emit(event.kind) {
                    for path in event.paths {
                        let _ = event_tx.send(path);
                    }
                }
            }
        })?;

        watcher.watch(workspace_root, RecursiveMode::Recursive)?;

        // Spawn the coalesce + broadcast task.
        tokio::spawn(async move {
            let mut pending: HashSet<PathBuf> = HashSet::new();

            loop {
                // Collect events that arrive within the throttle window.
                let deadline = sleep(THROTTLE);
                tokio::pin!(deadline);

                loop {
                    tokio::select! {
                        biased;
                        path = event_rx.recv() => {
                            match path {
                                Some(p) => { pending.insert(p); }
                                None => return, // channel closed, exit task
                            }
                        }
                        () = &mut deadline => break,
                    }
                }

                if !pending.is_empty() {
                    let paths: Vec<PathBuf> = pending.drain().collect();
                    tracing::debug!(count = paths.len(), "workspace files changed");
                    let _ = tx_clone.send(FileChangedEvent::WorkspaceChanged { paths });
                }
            }
        });

        Ok(Self {
            inner: Some(Mutex::new(InnerWatcher {
                _watcher: Box::new(watcher),
            })),
            tx,
        })
    }

    /// Create a no-op watcher that never broadcasts events.
    ///
    /// Useful in tests or when watching is intentionally disabled.
    #[must_use]
    pub fn noop() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { inner: None, tx }
    }

    /// Subscribe to file change events.
    ///
    /// The returned receiver may lag and produce `RecvError::Lagged` if events
    /// are generated faster than the subscriber consumes them.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<FileChangedEvent> {
        self.tx.subscribe()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` for Create, Modify, and Remove events; ignores Access/Other.
fn should_emit(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_does_not_panic() {
        let w = FileWatcher::noop();
        let _rx = w.subscribe();
    }

    #[test]
    fn should_emit_filters_correctly() {
        assert!(should_emit(EventKind::Create(
            notify::event::CreateKind::File
        )));
        assert!(should_emit(EventKind::Modify(
            notify::event::ModifyKind::Data(notify::event::DataChange::Content)
        )));
        assert!(should_emit(EventKind::Remove(
            notify::event::RemoveKind::File
        )));
        assert!(!should_emit(EventKind::Access(
            notify::event::AccessKind::Read
        )));
    }

    #[tokio::test]
    async fn new_watches_temp_dir() {
        let dir = std::env::temp_dir().join("rustcode-watcher-test");
        let _ = std::fs::create_dir_all(&dir);
        let watcher = FileWatcher::new(&dir);
        // If notify can't be initialised (e.g. CI with no inotify), skip.
        if let Ok(w) = watcher {
            let _rx = w.subscribe();
            // Just verify it doesn't panic; no events expected in this test.
        }
    }
}
