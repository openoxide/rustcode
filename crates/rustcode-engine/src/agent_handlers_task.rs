use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::sync::Mutex;

use rustcode_core::command::AgentOptions;
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::PublishError;
use rustcode_core::event::{Event, EventPayload};
use rustcode_core::ports::EventPublisher;

use super::ExecutionError;
use crate::Engine;

/// Publisher that collects `OutputChunk` events into a string buffer.
///
/// Used by the `task` tool to capture sub-agent output without forwarding
/// it to the parent session's event channel.
struct TextCollectPublisher {
    buf: Arc<Mutex<String>>,
}

#[async_trait]
impl EventPublisher for TextCollectPublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        if let EventPayload::OutputChunk { text } = event.payload {
            self.buf.lock().await.push_str(&text);
        }
        Ok(())
    }
}

impl Engine {
    /// Sub-agent delegation: run a child agent loop with the given prompt
    /// and return its final text output as a string.
    ///
    /// The child agent inherits permission flags from the parent options and
    /// shares the parent's cancellation token (cancelling the parent also
    /// cancels the sub-agent). Tool output is capped at 50 steps to prevent
    /// runaway recursion.
    ///
    /// # Errors
    /// Returns `ExecutionError::Dispatch` if the prompt is empty.
    /// Returns `ExecutionError::Cancelled` if the cancellation token fires.
    /// Returns `ExecutionError::Executor` if the sub-agent itself fails.
    pub(crate) async fn agent_tool_task(
        &self,
        prompt: &str,
        max_steps: Option<usize>,
        context: &CommandContext,
        parent_options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(ExecutionError::Dispatch(
                "task tool requires a non-empty prompt".to_string(),
            ));
        }

        // Cap sub-agent steps: default 20, max 50, never exceed parent
        let sub_max_steps = max_steps
            .unwrap_or(20)
            .min(50)
            .min(parent_options.max_steps);

        if sub_max_steps == 0 {
            return Err(ExecutionError::Dispatch(
                "task tool: max_steps must be > 0".to_string(),
            ));
        }

        let sub_options = AgentOptions {
            max_steps: sub_max_steps,
            allow_write: parent_options.allow_write,
            allow_edit: parent_options.allow_edit,
            allow_exec: parent_options.allow_exec,
            ..AgentOptions::default()
        };

        // Child context: shares config + a child cancellation token so that
        // cancelling the parent propagates to the sub-agent.
        let sub_session_meta = SessionMeta {
            session_id: format!("{}.subtask", context.session.session_id),
            request_id: format!("{}.subtask", context.session.request_id),
            started_at: SystemTime::now(),
        };
        let sub_context = CommandContext::with_cancellation(
            context.config.clone(),
            sub_session_meta,
            context.cancellation.child_token(),
        );

        // Collect sub-agent text output into a buffer
        let buf = Arc::new(Mutex::new(String::new()));
        let publisher: Arc<dyn EventPublisher> =
            Arc::new(TextCollectPublisher { buf: buf.clone() });

        self.run_agent(
            prompt.to_string(),
            sub_options,
            Vec::new(),
            &sub_context,
            publisher,
        )
        .await?;

        let output = buf.lock().await.clone();
        if output.is_empty() {
            Ok("[sub-agent completed successfully with no text output]".to_string())
        } else {
            Ok(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_core::event::{EventScope, EVENT_SCHEMA_VERSION};

    #[tokio::test]
    async fn text_collect_publisher_collects_output_chunks() {
        let buf = Arc::new(Mutex::new(String::new()));
        let publisher = TextCollectPublisher { buf: buf.clone() };

        publisher
            .publish(Event {
                schema_version: EVENT_SCHEMA_VERSION,
                id: 1,
                timestamp: SystemTime::now(),
                scope: EventScope::Command,
                payload: EventPayload::OutputChunk {
                    text: "hello ".to_string(),
                },
            })
            .await
            .unwrap();

        publisher
            .publish(Event {
                schema_version: EVENT_SCHEMA_VERSION,
                id: 2,
                timestamp: SystemTime::now(),
                scope: EventScope::Command,
                payload: EventPayload::OutputChunk {
                    text: "world".to_string(),
                },
            })
            .await
            .unwrap();

        // Non-chunk events are ignored
        publisher
            .publish(Event {
                schema_version: EVENT_SCHEMA_VERSION,
                id: 3,
                timestamp: SystemTime::now(),
                scope: EventScope::System,
                payload: EventPayload::Completed,
            })
            .await
            .unwrap();

        assert_eq!(&*buf.lock().await, "hello world");
    }
}
