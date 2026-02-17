use thiserror::Error;
use tokio::sync::mpsc;

use rustcode_core::event::Event;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("ui channel closed")]
    ChannelClosed,
}

pub struct TuiApp {
    receiver: mpsc::Receiver<Event>,
}

impl TuiApp {
    pub fn new(receiver: mpsc::Receiver<Event>) -> Self {
        Self { receiver }
    }

    pub async fn run(mut self) -> Result<(), TuiError> {
        while let Some(_event) = self.receiver.recv().await {}
        Ok(())
    }
}
