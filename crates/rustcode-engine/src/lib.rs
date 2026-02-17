use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use rustcode_core::command::Command;
use rustcode_core::context::CommandContext;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::ports::{CommandExecutor, EventPublisher};
use rustcode_io::{ProcessOutput, ProcessPort};
use rustcode_llm::{LlmClient, LlmRequest};
use rustcode_plugins::PluginRegistry;

#[derive(Clone)]
pub struct ChannelPublisher {
    sender: mpsc::Sender<Event>,
}

impl ChannelPublisher {
    pub fn new(sender: mpsc::Sender<Event>) -> Self {
        Self { sender }
    }
}

#[async_trait]
impl EventPublisher for ChannelPublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        self.sender
            .send(event)
            .await
            .map_err(|_| PublishError::SinkClosed)
    }
}

pub struct Engine {
    llm: Arc<dyn LlmClient>,
    process: Arc<dyn ProcessPort>,
    plugins: PluginRegistry,
    next_event_id: AtomicU64,
}

impl Engine {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        process: Arc<dyn ProcessPort>,
        plugins: PluginRegistry,
    ) -> Self {
        Self {
            llm,
            process,
            plugins,
            next_event_id: AtomicU64::new(1),
        }
    }

    async fn emit(
        &self,
        publisher: Arc<dyn EventPublisher>,
        scope: EventScope,
        payload: EventPayload,
        context: &CommandContext,
    ) -> Result<(), ExecutionError> {
        let event = Event::new(
            self.next_event_id.fetch_add(1, Ordering::Relaxed),
            scope,
            payload,
        );

        publisher
            .publish(event.clone())
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        for plugin in self.plugins.plugins() {
            plugin
                .on_event(&event, context)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        }

        Ok(())
    }

    async fn run_exec(
        &self,
        command: String,
        args: Vec<String>,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let ProcessOutput { stdout, .. } = self
            .process
            .run(&command, &args, &context.config.workspace_root)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        self.emit(
            publisher,
            EventScope::Tool,
            EventPayload::OutputChunk { text: stdout },
            context,
        )
        .await
    }

    async fn run_prompt(
        &self,
        prompt: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let response = self
            .llm
            .complete(LlmRequest {
                model: context.config.model.clone(),
                prompt,
            })
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        self.emit(
            publisher,
            EventScope::Command,
            EventPayload::OutputChunk {
                text: response.text,
            },
            context,
        )
        .await
    }
}

#[async_trait]
impl CommandExecutor for Engine {
    async fn execute(
        &self,
        command: Command,
        context: CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        debug!(?command, "engine command received");

        let command_name = format!("{command:?}");
        self.emit(
            publisher.clone(),
            EventScope::System,
            EventPayload::CommandAccepted { name: command_name },
            &context,
        )
        .await?;

        match command {
            Command::Run { prompt } => self.run_prompt(prompt, &context, publisher.clone()).await?,
            Command::Exec { command, args } => {
                self.run_exec(command, args, &context, publisher.clone())
                    .await?
            }
            Command::Tui => {
                self.emit(
                    publisher.clone(),
                    EventScope::Ui,
                    EventPayload::Warning {
                        message: "TUI handoff requested".to_string(),
                    },
                    &context,
                )
                .await?;
            }
            Command::Serve { listen } => {
                self.emit(
                    publisher.clone(),
                    EventScope::System,
                    EventPayload::Warning {
                        message: format!("serve endpoint configured: {listen}"),
                    },
                    &context,
                )
                .await?;
            }
            Command::Version => {
                self.emit(
                    publisher.clone(),
                    EventScope::System,
                    EventPayload::OutputChunk {
                        text: env!("CARGO_PKG_VERSION").to_string(),
                    },
                    &context,
                )
                .await?;
            }
        }

        self.emit(
            publisher,
            EventScope::System,
            EventPayload::Completed,
            &context,
        )
        .await
    }
}
