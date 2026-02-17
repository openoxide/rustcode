use thiserror::Error;
use tokio::sync::mpsc;

use rustcode_core::event::Event;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("ui channel closed")]
    ChannelClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSummary {
    pub events_processed: usize,
}

pub struct TuiApp {
    receiver: mpsc::Receiver<Event>,
}

impl TuiApp {
    pub fn new(receiver: mpsc::Receiver<Event>) -> Self {
        Self { receiver }
    }

    pub async fn run(mut self) -> Result<UiSummary, TuiError> {
        let mut events_processed = 0usize;
        while let Some(_event) = self.receiver.recv().await {
            events_processed += 1;
        }
        Ok(UiSummary { events_processed })
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use rustcode_core::event::{Event, EventPayload, EventScope};

    use super::*;

    #[tokio::test]
    async fn run_consumes_all_events_until_channel_close() {
        let (tx, rx) = mpsc::channel(4);
        tx.send(Event {
            schema_version: 1,
            id: 1,
            timestamp: UNIX_EPOCH,
            scope: EventScope::System,
            payload: EventPayload::Completed,
        })
        .await
        .expect("must send event");
        tx.send(Event {
            schema_version: 1,
            id: 2,
            timestamp: UNIX_EPOCH,
            scope: EventScope::System,
            payload: EventPayload::Warning {
                message: "warning".to_string(),
            },
        })
        .await
        .expect("must send event");
        drop(tx);

        let summary = TuiApp::new(rx).run().await.expect("run should succeed");
        assert_eq!(summary.events_processed, 2);
    }
}
