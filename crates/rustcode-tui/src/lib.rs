use thiserror::Error;
use tokio::sync::mpsc;

use rustcode_core::event::Event;

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

pub use interactive::run_interactive;

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
