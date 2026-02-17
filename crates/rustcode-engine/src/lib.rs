use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::{
    env,
    path::{Component, Path, PathBuf},
};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::debug;

use rustcode_core::command::Command;
use rustcode_core::context::CommandContext;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::ports::{CommandExecutor, EventPublisher, PathOperation, PermissionPolicy};
use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
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

#[derive(Debug, Default)]
pub struct WorkspacePermissionPolicy;

impl PermissionPolicy for WorkspacePermissionPolicy {
    fn allow_path(
        &self,
        workspace_root: &Path,
        candidate: &Path,
        _operation: PathOperation,
    ) -> Result<(), ExecutionError> {
        if candidate.starts_with(workspace_root) {
            Ok(())
        } else {
            Err(ExecutionError::Dispatch(format!(
                "path escapes workspace root: {}",
                candidate.display()
            )))
        }
    }
}

pub struct Engine {
    llm: Arc<dyn LlmClient>,
    fs: Arc<dyn FileSystemPort>,
    process: Arc<dyn ProcessPort>,
    permission_policy: Arc<dyn PermissionPolicy>,
    plugins: PluginRegistry,
    next_event_id: AtomicU64,
}

impl Engine {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        fs: Arc<dyn FileSystemPort>,
        process: Arc<dyn ProcessPort>,
        permission_policy: Arc<dyn PermissionPolicy>,
        plugins: PluginRegistry,
    ) -> Self {
        Self {
            llm,
            fs,
            process,
            permission_policy,
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
            .run(
                &command,
                &args,
                &context.config.workspace_root,
                context.cancellation.clone(),
            )
            .await
            .map_err(|err| match err {
                IoError::Cancelled => ExecutionError::Cancelled,
                _ => ExecutionError::Executor(err.to_string()),
            })?;

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

    async fn run_list(
        &self,
        path: Option<String>,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let target = path.unwrap_or_else(|| ".".to_string());
        let resolved = self.resolve_workspace_path(context, &target, PathOperation::List)?;
        let entries = self
            .fs
            .list_dir(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        let workspace_root = absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        for entry in entries {
            let relative = entry
                .strip_prefix(&workspace_root)
                .unwrap_or(&entry)
                .display()
                .to_string();
            rendered.push_str(&relative);
            rendered.push('\n');
        }

        self.emit(
            publisher,
            EventScope::Tool,
            EventPayload::OutputChunk { text: rendered },
            context,
        )
        .await
    }

    async fn run_read(
        &self,
        path: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let resolved = self.resolve_workspace_path(context, &path, PathOperation::Read)?;
        let contents = self
            .fs
            .read_to_string(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        self.emit(
            publisher,
            EventScope::Tool,
            EventPayload::OutputChunk { text: contents },
            context,
        )
        .await
    }

    async fn run_write(
        &self,
        path: String,
        contents: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let resolved = self.resolve_workspace_path(context, &path, PathOperation::Write)?;
        self.fs
            .write_string(&resolved, &contents)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        self.emit(
            publisher,
            EventScope::Tool,
            EventPayload::OutputChunk {
                text: format!("wrote {} bytes to {}", contents.len(), resolved.display()),
            },
            context,
        )
        .await
    }

    async fn run_edit(
        &self,
        path: String,
        from: String,
        to: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let resolved = self.resolve_workspace_path(context, &path, PathOperation::Edit)?;
        let original = self
            .fs
            .read_to_string(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let updated = original.replace(&from, &to);
        self.fs
            .write_string(&resolved, &updated)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let changed = if original == updated { 0 } else { 1 };
        self.emit(
            publisher,
            EventScope::Tool,
            EventPayload::OutputChunk {
                text: format!(
                    "edit applied ({changed} replacement groups) to {}",
                    resolved.display()
                ),
            },
            context,
        )
        .await
    }

    fn resolve_workspace_path(
        &self,
        context: &CommandContext,
        requested: &str,
        operation: PathOperation,
    ) -> Result<PathBuf, ExecutionError> {
        let root = absolute_normalized(&context.config.workspace_root).map_err(|err| {
            ExecutionError::Dispatch(format!(
                "failed to resolve workspace root {}: {err}",
                context.config.workspace_root.display()
            ))
        })?;

        let candidate = PathBuf::from(requested);
        let joined = if candidate.is_absolute() {
            candidate
        } else {
            root.join(candidate)
        };
        let normalized = lexical_normalize(joined);

        self.permission_policy
            .allow_path(&root, &normalized, operation)?;

        Ok(normalized)
    }
}

fn absolute_normalized(path: &Path) -> Result<PathBuf, std::io::Error> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    Ok(lexical_normalize(absolute))
}

fn lexical_normalize(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }

    normalized
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

        let command_result = match command {
            Command::Run { prompt } => self.run_prompt(prompt, &context, publisher.clone()).await,
            Command::Exec { command, args } => {
                self.run_exec(command, args, &context, publisher.clone())
                    .await
            }
            Command::List { path } => self.run_list(path, &context, publisher.clone()).await,
            Command::Read { path } => self.run_read(path, &context, publisher.clone()).await,
            Command::Write { path, contents } => {
                self.run_write(path, contents, &context, publisher.clone())
                    .await
            }
            Command::Edit { path, from, to } => {
                self.run_edit(path, from, to, &context, publisher.clone())
                    .await
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
                .await
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
                .await
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
                .await
            }
        };

        match command_result {
            Ok(()) => {
                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::Completed,
                    &context,
                )
                .await
            }
            Err(ExecutionError::Cancelled) => {
                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::Warning {
                        message: "execution cancelled".to_string(),
                    },
                    &context,
                )
                .await?;
                Err(ExecutionError::Cancelled)
            }
            Err(err) => {
                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::Failure {
                        message: err.to_string(),
                    },
                    &context,
                )
                .await?;
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::SystemTime;

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use rustcode_core::config::ResolvedConfig;
    use rustcode_core::context::{CommandContext, SessionMeta};
    use rustcode_core::error::PublishError;
    use rustcode_core::event::{Event, EventPayload};
    use rustcode_core::ports::EventPublisher;
    use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
    use rustcode_llm::NullLlmClient;
    use rustcode_plugins::{Plugin, PluginError, PluginRegistry};

    use super::*;

    struct CancelledProcess;
    struct DummyFs;

    #[async_trait]
    impl FileSystemPort for DummyFs {
        async fn read_to_string(&self, _path: &Path) -> Result<String, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
            Err(IoError::Io("not used".to_string()))
        }
    }

    #[async_trait]
    impl ProcessPort for CancelledProcess {
        async fn run(
            &self,
            _program: &str,
            _args: &[String],
            _cwd: &Path,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, IoError> {
            Err(IoError::Cancelled)
        }
    }

    #[derive(Default)]
    struct CollectingPublisher {
        events: Mutex<Vec<Event>>,
    }

    struct CountingPlugin {
        seen: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl Plugin for CountingPlugin {
        fn name(&self) -> &'static str {
            "counting"
        }

        async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
            let mut seen = self.seen.lock().await;
            *seen += 1;
            Ok(())
        }
    }

    #[async_trait]
    impl EventPublisher for CollectingPublisher {
        async fn publish(&self, event: Event) -> Result<(), PublishError> {
            self.events.lock().await.push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancellation_emits_warning_and_returns_cancelled() {
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig::default()),
            SessionMeta {
                session_id: "s1".to_string(),
                request_id: "r1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine
            .execute(
                Command::Exec {
                    command: "sleep".to_string(),
                    args: vec!["30".to_string()],
                },
                context,
                publisher.clone(),
            )
            .await;

        assert!(matches!(result, Err(ExecutionError::Cancelled)));

        let events = publisher.events.lock().await.clone();
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::CommandAccepted { name }
                if name.contains("Exec")
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::Warning { message }
                if message == "execution cancelled"
            )
        }));
        assert!(!events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::Completed)));
    }

    #[tokio::test]
    async fn read_rejects_workspace_escape() {
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                workspace_root: PathBuf::from("/tmp/rustcode-workspace"),
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "s2".to_string(),
                request_id: "r2".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine
            .execute(
                Command::Read {
                    path: "../secret.txt".to_string(),
                },
                context,
                publisher.clone(),
            )
            .await;

        assert!(matches!(result, Err(ExecutionError::Dispatch(_))));

        let events = publisher.events.lock().await.clone();
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::Failure { message }
                if message.contains("path escapes workspace root")
            )
        }));
    }

    #[tokio::test]
    async fn plugins_receive_emitted_events() {
        let seen = Arc::new(Mutex::new(0usize));
        let mut registry = PluginRegistry::default();
        registry
            .register(Arc::new(CountingPlugin { seen: seen.clone() }))
            .expect("plugin registration must succeed");

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            registry,
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig::default()),
            SessionMeta {
                session_id: "s3".to_string(),
                request_id: "r3".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine.execute(Command::Version, context, publisher).await;
        assert!(result.is_ok());

        let seen_count = *seen.lock().await;
        assert_eq!(seen_count, 3);
    }
}
