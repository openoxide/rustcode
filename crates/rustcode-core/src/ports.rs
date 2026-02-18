use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::command::Command;
use crate::context::CommandContext;
use crate::error::{ExecutionError, PublishError};
use crate::event::Event;
use crate::session::StoredMessage;
use crate::tool_approval::ToolApprovalRequest;

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
    /// Validates that `candidate` is allowed under `workspace_root` for the given operation.
    ///
    /// # Errors
    /// Returns an `ExecutionError` when the operation is not permitted.
    fn allow_path(
        &self,
        workspace_root: &Path,
        candidate: &Path,
        operation: PathOperation,
    ) -> Result<(), ExecutionError>;
}

#[async_trait]
pub trait TranscriptRecorder: Send + Sync {
    /// Append a single transcript message to a session.
    ///
    /// Implementations should be durable and should not reorder messages.
    async fn append_message(
        &self,
        session_id: &str,
        message: StoredMessage,
    ) -> Result<(), ExecutionError>;
}

#[async_trait]
pub trait ToolApprover: Send + Sync {
    async fn approve(&self, request: ToolApprovalRequest) -> Result<bool, ExecutionError>;
}
