use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use rustcode_core::config::ResolvedConfig;
use rustcode_core::event::Event;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::ports::{CommandExecutor, EventPublisher, ToolApprover};
use rustcode_core::tool_approval::ToolApprovalRequest;
use rustcode_core::SessionInfo;
use rustcode_state::SessionStore;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("ui channel closed")]
    ChannelClosed,
    #[error("io error: {0}")]
    Io(String),
    #[error("state error: {0}")]
    State(String),
}

mod interactive;

#[derive(Debug, Clone)]
pub struct InteractiveDefaults {
    pub workspace_root: PathBuf,
    pub model: String,
}

#[derive(Debug, Clone)]
pub enum InteractiveStart {
    Sessions,
    Chat {
        session: SessionInfo,
        prompt: Option<String>,
        auto_submit: bool,
    },
}

#[derive(Debug)]
pub enum InteractiveMsg {
    EngineEvent(Event),
    RunEnded {
        ok: bool,
        message: Option<String>,
    },
    ApprovalRequest {
        request: ToolApprovalRequest,
        reply: oneshot::Sender<bool>,
    },
}

pub struct InteractiveHandles {
    pub tx: mpsc::UnboundedSender<InteractiveMsg>,
    pub rx: mpsc::UnboundedReceiver<InteractiveMsg>,
    pub approver: Arc<dyn ToolApprover>,
}

impl InteractiveHandles {
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx: tx.clone(),
            rx,
            approver: Arc::new(TuiToolApprover { tx }),
        }
    }
}

#[derive(Clone)]
pub struct TuiPublisher {
    tx: mpsc::UnboundedSender<InteractiveMsg>,
}

impl TuiPublisher {
    #[must_use]
    pub fn new(tx: mpsc::UnboundedSender<InteractiveMsg>) -> Self {
        Self { tx }
    }
}

#[async_trait::async_trait]
impl EventPublisher for TuiPublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        self.tx
            .send(InteractiveMsg::EngineEvent(event))
            .map_err(|_| PublishError::SinkClosed)
    }
}

struct TuiToolApprover {
    tx: mpsc::UnboundedSender<InteractiveMsg>,
}

#[async_trait::async_trait]
impl ToolApprover for TuiToolApprover {
    async fn approve(&self, request: ToolApprovalRequest) -> Result<bool, ExecutionError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(InteractiveMsg::ApprovalRequest { request, reply: tx })
            .map_err(|_| ExecutionError::Executor("approval channel closed".to_string()))?;
        rx.await
            .map_err(|_| ExecutionError::Executor("approval response dropped".to_string()))
    }
}

pub struct InteractiveServices {
    pub store: SessionStore,
    pub defaults: InteractiveDefaults,
    pub initial_status: Option<String>,
    pub start: InteractiveStart,
    pub runtime: Handle,
    pub handles: InteractiveHandles,

    pub config: Option<Arc<ResolvedConfig>>,
    pub executor: Option<Arc<dyn CommandExecutor>>,
}

pub fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    interactive::run_interactive(services)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiInput {
    Domain(Event),
    Resize { width: u16, height: u16 },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSummary {
    pub events_processed: usize,
    pub resize_events: usize,
    pub shutdown_received: bool,
}

pub struct TuiApp {
    receiver: mpsc::Receiver<UiInput>,
}

impl TuiApp {
    #[must_use]
    pub fn new(receiver: mpsc::Receiver<UiInput>) -> Self {
        Self { receiver }
    }

    #[must_use]
    pub fn from_domain_receiver(mut receiver: mpsc::Receiver<Event>) -> Self {
        let (ui_tx, ui_rx) = mpsc::channel(512);

        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                if ui_tx.send(UiInput::Domain(event)).await.is_err() {
                    return;
                }
            }
            let _ = ui_tx.send(UiInput::Shutdown).await;
        });

        Self::new(ui_rx)
    }

    /// Run the TUI event loop.
    ///
    /// # Errors
    /// Returns `TuiError::ChannelClosed` if the input channel closes without an explicit
    /// `UiInput::Shutdown` message.
    pub async fn run(mut self) -> Result<UiSummary, TuiError> {
        let mut events_processed = 0usize;
        let mut resize_events = 0usize;
        let mut shutdown_received = false;

        while let Some(event) = self.receiver.recv().await {
            match event {
                UiInput::Domain(_event) => {
                    events_processed += 1;
                }
                UiInput::Resize { .. } => {
                    resize_events += 1;
                }
                UiInput::Shutdown => {
                    shutdown_received = true;
                    break;
                }
            }
        }

        if !shutdown_received {
            return Err(TuiError::ChannelClosed);
        }

        Ok(UiSummary {
            events_processed,
            resize_events,
            shutdown_received,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use rustcode_core::event::{Event, EventPayload, EventScope};

    use super::*;

    #[tokio::test]
    async fn run_consumes_domain_and_resize_until_shutdown() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(UiInput::Domain(Event {
            schema_version: 1,
            id: 1,
            timestamp: UNIX_EPOCH,
            scope: EventScope::System,
            payload: EventPayload::Completed,
        }))
        .await
        .expect("must send event");
        tx.send(UiInput::Resize {
            width: 120,
            height: 40,
        })
        .await
        .expect("must send resize");
        tx.send(UiInput::Shutdown)
            .await
            .expect("must send shutdown");

        let summary = TuiApp::new(rx).run().await.expect("run should succeed");
        assert_eq!(summary.events_processed, 1);
        assert_eq!(summary.resize_events, 1);
        assert!(summary.shutdown_received);
    }

    #[tokio::test]
    async fn adapter_converts_domain_stream_and_adds_shutdown() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(Event {
            schema_version: 1,
            id: 7,
            timestamp: UNIX_EPOCH,
            scope: EventScope::System,
            payload: EventPayload::Completed,
        })
        .await
        .expect("must send event");
        drop(tx);

        let summary = TuiApp::from_domain_receiver(rx)
            .run()
            .await
            .expect("run should succeed");
        assert_eq!(summary.events_processed, 1);
        assert!(summary.shutdown_received);
    }
}
