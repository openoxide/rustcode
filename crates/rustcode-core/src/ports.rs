use std::sync::Arc;

use async_trait::async_trait;

use crate::command::Command;
use crate::context::CommandContext;
use crate::error::{ExecutionError, PublishError};
use crate::event::Event;

#[async_trait]
pub trait EventPublisher: Send + Sync {
    async fn publish(&self, event: Event) -> Result<(), PublishError>;
}

#[async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute(
        &self,
        command: Command,
        context: CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError>;
}
