use std::path::Path;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOperation {
    List,
    Read,
    Write,
    Edit,
}

pub trait PermissionPolicy: Send + Sync {
    fn allow_path(
        &self,
        workspace_root: &Path,
        candidate: &Path,
        operation: PathOperation,
    ) -> Result<(), ExecutionError>;
}
