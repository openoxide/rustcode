use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::{
    env,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;

use rustcode_core::command::Command;
use rustcode_core::context::CommandContext;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::ports::{CommandExecutor, EventPublisher, PathOperation, PermissionPolicy};
use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
use rustcode_llm::{ChatMessage, ChatRequest, ChatRole, LlmClient, LlmRequest, ToolSpec};
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
        let response = tokio::select! {
            _ = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            result = self.llm.complete(LlmRequest {
                model: context.config.model.clone(),
                prompt,
            }) => {
                result
            }
        }
        .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        if response.chunks.is_empty() {
            self.emit(
                publisher,
                EventScope::Command,
                EventPayload::OutputChunk {
                    text: response.text,
                },
                context,
            )
            .await?;
            return Ok(());
        }

        for chunk in response.chunks {
            self.emit(
                publisher.clone(),
                EventScope::Command,
                EventPayload::OutputChunk { text: chunk },
                context,
            )
            .await?;
        }

        Ok(())
    }

    async fn run_agent(
        &self,
        prompt: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        const MAX_STEPS: usize = 8;
        const MAX_TOOL_CALLS_PER_STEP: usize = 8;

        let tools = agent_tool_specs();
        let mut messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: Value::String(
                    "You are rustcode, a production-grade coding agent.\n\
Use tools when you need filesystem context.\n\
Prefer: list -> read.\n\
Only modify files via write/edit when explicitly required.\n\
When you are done, respond with a final plain-text answer."
                        .to_string(),
                ),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: Value::String(prompt),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
        ];

        for _step in 0..MAX_STEPS {
            let request = ChatRequest {
                model: context.config.model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
            };

            let response = tokio::select! {
                _ = context.cancellation.cancelled() => {
                    return Err(ExecutionError::Cancelled);
                }
                result = self.llm.chat(request) => {
                    result
                }
            }
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

            // Preserve assistant tool_calls in the transcript before we emit tool results.
            messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: if response.text.is_empty() {
                    Value::Null
                } else {
                    Value::String(response.text.clone())
                },
                tool_call_id: None,
                tool_calls: response.tool_calls.clone(),
            });

            if !response.text.is_empty() {
                self.emit(
                    publisher.clone(),
                    EventScope::Command,
                    EventPayload::OutputChunk {
                        text: response.text.clone(),
                    },
                    context,
                )
                .await?;
            }

            if response.tool_calls.is_empty() {
                return Ok(());
            }

            for call in response.tool_calls.iter().take(MAX_TOOL_CALLS_PER_STEP) {
                self.emit(
                    publisher.clone(),
                    EventScope::Tool,
                    EventPayload::ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                    context,
                )
                .await?;

                let tool_result = self
                    .execute_agent_tool_call(call.name.as_str(), call.arguments.as_str(), context)
                    .await;

                let (ok, output) = match tool_result {
                    Ok(output) => (true, output),
                    Err(err) => (false, err.to_string()),
                };
                let result_payload = json!({
                    "ok": ok,
                    "output": output,
                })
                .to_string();

                self.emit(
                    publisher.clone(),
                    EventScope::Tool,
                    EventPayload::ToolResult {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        ok,
                        output: result_payload.clone(),
                    },
                    context,
                )
                .await?;

                messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: Value::String(result_payload),
                    tool_call_id: Some(call.id.clone()),
                    tool_calls: Vec::new(),
                });
            }
        }

        Err(ExecutionError::Executor(
            "agent exceeded maximum tool loop steps".to_string(),
        ))
    }

    async fn execute_agent_tool_call(
        &self,
        name: &str,
        arguments: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let args: Value = serde_json::from_str(arguments).map_err(|err| {
            ExecutionError::Dispatch(format!("tool arguments are not valid JSON: {err}"))
        })?;

        match name {
            "list" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                self.agent_tool_list(path, context).await
            }
            "read" => {
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("read tool requires path".to_string())
                })?;
                self.agent_tool_read(path, context).await
            }
            "write" => {
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("write tool requires path".to_string())
                })?;
                let contents = args
                    .get("contents")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ExecutionError::Dispatch("write tool requires contents".to_string())
                    })?;
                self.agent_tool_write(path, contents, context).await
            }
            "edit" => {
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("edit tool requires path".to_string())
                })?;
                let from = args.get("from").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("edit tool requires from".to_string())
                })?;
                let to = args
                    .get("to")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ExecutionError::Dispatch("edit tool requires to".to_string()))?;
                self.agent_tool_edit(path, from, to, context).await
            }
            _ => Err(ExecutionError::Dispatch(format!(
                "unknown tool call: {name}"
            ))),
        }
    }

    async fn agent_tool_list(
        &self,
        path: Option<String>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
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
        Ok(rendered)
    }

    async fn agent_tool_read(
        &self,
        path: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Read)?;
        self.fs
            .read_to_string(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))
    }

    async fn agent_tool_write(
        &self,
        path: &str,
        contents: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Write)?;
        self.fs
            .write_string(&resolved, contents)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        Ok(format!(
            "wrote {} bytes to {}",
            contents.len(),
            resolved.display()
        ))
    }

    async fn agent_tool_edit(
        &self,
        path: &str,
        from: &str,
        to: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Edit)?;
        let original = self
            .fs
            .read_to_string(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let updated = original.replace(from, to);
        self.fs
            .write_string(&resolved, &updated)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let changed = if original == updated { 0 } else { 1 };
        Ok(format!(
            "edit applied ({changed} replacement groups) to {}",
            resolved.display()
        ))
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

    async fn run_serve(
        &self,
        listen: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let listener = TcpListener::bind(&listen)
            .await
            .map_err(|err| ExecutionError::Executor(format!("failed to bind {listen}: {err}")))?;
        let bound_addr = listener.local_addr().map_err(|err| {
            ExecutionError::Executor(format!("failed to inspect bind addr: {err}"))
        })?;

        self.emit(
            publisher.clone(),
            EventScope::System,
            EventPayload::Warning {
                message: format!("serve endpoint configured: {bound_addr}"),
            },
            context,
        )
        .await?;

        loop {
            tokio::select! {
                _ = context.cancellation.cancelled() => {
                    return Err(ExecutionError::Cancelled);
                }
                incoming = listener.accept() => {
                    match incoming {
                        Ok((stream, _addr)) => {
                            if let Err(err) = self
                                .handle_serve_connection(stream, context, publisher.clone())
                                .await
                            {
                                self.emit(
                                    publisher.clone(),
                                    EventScope::System,
                                    EventPayload::Warning {
                                        message: format!("serve connection error: {err}"),
                                    },
                                    context,
                                )
                                .await?;
                            }
                        }
                        Err(err) => {
                            return Err(ExecutionError::Executor(format!("accept failed: {err}")));
                        }
                    }
                }
            }
        }
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

    async fn handle_serve_connection(
        &self,
        mut stream: TcpStream,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let mut buffer = [0u8; 2048];
        let bytes = match timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(err)) => {
                return Err(ExecutionError::Executor(format!(
                    "failed to read request: {err}"
                )));
            }
            Err(_) => {
                let body = "{\"error\":\"request timeout\"}\n";
                let response = format!(
                    "HTTP/1.1 408 Request Timeout\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );

                stream.write_all(response.as_bytes()).await.map_err(|err| {
                    ExecutionError::Executor(format!("failed to write timeout response: {err}"))
                })?;
                stream.shutdown().await.map_err(|err| {
                    ExecutionError::Executor(format!("failed to shutdown stream: {err}"))
                })?;

                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::ServeRequest {
                        method: String::new(),
                        path: String::new(),
                        status: 408,
                    },
                    context,
                )
                .await?;
                return Ok(());
            }
        };

        let request = String::from_utf8_lossy(&buffer[..bytes]);
        let request_line = request.lines().next().unwrap_or_default();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();

        let (status_code, status_line, body) = match (method.as_str(), path.as_str()) {
            ("GET", "/health") => (200u16, "200 OK", "{\"ok\":true}\n"),
            ("", "") => (400u16, "400 Bad Request", "{\"error\":\"bad request\"}\n"),
            _ => (404u16, "404 Not Found", "{\"error\":\"not found\"}\n"),
        };

        let response = format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );

        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|err| ExecutionError::Executor(format!("failed to write response: {err}")))?;
        stream
            .shutdown()
            .await
            .map_err(|err| ExecutionError::Executor(format!("failed to shutdown stream: {err}")))?;

        self.emit(
            publisher,
            EventScope::System,
            EventPayload::ServeRequest {
                method,
                path,
                status: status_code,
            },
            context,
        )
        .await
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
            Command::Agent { prompt } => self.run_agent(prompt, &context, publisher.clone()).await,
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
            Command::Serve { listen } => self.run_serve(listen, &context, publisher.clone()).await,
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

fn agent_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "list".to_string(),
            description: "List files and directories under a workspace-relative path.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative path (default: .)" }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "read".to_string(),
            description: "Read a UTF-8 text file from the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "write".to_string(),
            description: "Write a UTF-8 text file to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" },
                    "contents": { "type": "string", "description": "Full file contents" }
                },
                "required": ["path", "contents"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "edit".to_string(),
            description: "Replace a substring in a workspace file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" },
                    "from": { "type": "string", "description": "Exact text to replace" },
                    "to": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "from", "to"],
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use rustcode_core::config::ResolvedConfig;
    use rustcode_core::context::{CommandContext, SessionMeta};
    use rustcode_core::error::PublishError;
    use rustcode_core::event::{Event, EventPayload};
    use rustcode_core::ports::EventPublisher;
    use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
    use rustcode_llm::{
        ChatRequest, ChatResponse, LlmClient, LlmRequest, LlmResponse, NullLlmClient, ToolCall,
    };
    use rustcode_plugins::{Plugin, PluginError, PluginRegistry};

    use super::*;

    struct CancelledProcess;
    struct DummyFs;
    struct StreamingLlmClient;
    struct AgentFs {
        root: PathBuf,
    }
    struct ScriptedAgentLlm {
        step: Mutex<usize>,
    }

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
    impl FileSystemPort for AgentFs {
        async fn read_to_string(&self, _path: &Path) -> Result<String, IoError> {
            Ok("agent-read-ok".to_string())
        }

        async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
            Ok(())
        }

        async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
            Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
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

    #[async_trait]
    impl LlmClient for StreamingLlmClient {
        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<LlmResponse, rustcode_llm::LlmError> {
            Ok(LlmResponse {
                text: "hello world".to_string(),
                chunks: vec!["hello".to_string(), " world".to_string()],
            })
        }
    }

    #[async_trait]
    impl LlmClient for ScriptedAgentLlm {
        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<LlmResponse, rustcode_llm::LlmError> {
            Ok(LlmResponse {
                text: "unused".to_string(),
                chunks: Vec::new(),
            })
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, rustcode_llm::LlmError> {
            let mut step = self.step.lock().await;
            match *step {
                0 => {
                    *step = 1;
                    assert!(
                        !request.tools.is_empty(),
                        "agent request must include tools"
                    );
                    Ok(ChatResponse {
                        text: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call_1".to_string(),
                            name: "list".to_string(),
                            arguments: r#"{"path":"."}"#.to_string(),
                        }],
                    })
                }
                _ => {
                    // Ensure the tool result was appended before the second turn.
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|msg| msg.tool_call_id.as_deref() == Some("call_1")),
                        "expected tool result message for call_1"
                    );
                    Ok(ChatResponse {
                        text: "done".to_string(),
                        tool_calls: Vec::new(),
                    })
                }
            }
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
    async fn agent_executes_tool_calls_and_emits_tool_events() {
        let workspace_root = PathBuf::from("/tmp/rustcode-agent-workspace");
        let engine = Engine::new(
            Arc::new(ScriptedAgentLlm {
                step: Mutex::new(0),
            }),
            Arc::new(AgentFs {
                root: workspace_root.clone(),
            }),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                workspace_root,
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "agent-s1".to_string(),
                request_id: "agent-r1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine
            .execute(
                Command::Agent {
                    prompt: "hi".to_string(),
                },
                context,
                publisher.clone(),
            )
            .await;
        assert!(result.is_ok(), "result={result:?}");

        let events = publisher.events.lock().await.clone();
        assert!(events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::ToolCall { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::ToolResult { .. })));
        assert!(events.iter().any(|event| matches!(event.payload, EventPayload::OutputChunk { ref text } if text.contains("done"))));
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

    #[tokio::test]
    async fn run_prompt_emits_streaming_chunks_when_available() {
        let engine = Engine::new(
            Arc::new(StreamingLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig::default()),
            SessionMeta {
                session_id: "s-stream".to_string(),
                request_id: "r-stream".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine
            .execute(
                Command::Run {
                    prompt: "stream".to_string(),
                },
                context,
                publisher.clone(),
            )
            .await;
        assert!(result.is_ok());

        let events = publisher.events.lock().await.clone();
        let chunks: Vec<String> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::OutputChunk { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(chunks, vec!["hello".to_string(), " world".to_string()]);
    }

    #[tokio::test]
    async fn serve_waits_until_cancelled() {
        let cancellation = CancellationToken::new();
        let context = CommandContext::with_cancellation(
            Arc::new(ResolvedConfig::default()),
            SessionMeta {
                session_id: "s4".to_string(),
                request_id: "r4".to_string(),
                started_at: SystemTime::now(),
            },
            cancellation.clone(),
        );

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
        );
        let publisher = Arc::new(CollectingPublisher::default());

        let task = tokio::spawn({
            let publisher = publisher.clone();
            async move {
                engine
                    .execute(
                        Command::Serve {
                            listen: "127.0.0.1:4317".to_string(),
                        },
                        context,
                        publisher,
                    )
                    .await
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation.cancel();

        let result = task.await.expect("task must join");
        let events = publisher.events.lock().await.clone();
        match result {
            Err(ExecutionError::Cancelled) => {
                assert!(events.iter().any(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::Warning { message }
                        if message.contains("serve endpoint configured")
                    )
                }));
                assert!(events.iter().any(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::Warning { message }
                        if message == "execution cancelled"
                    )
                }));
            }
            Err(ExecutionError::Executor(message)) if message.contains("failed to bind") => {
                // Some restricted environments disallow local listener binds.
                assert!(events.iter().any(|event| {
                    matches!(
                        &event.payload,
                        EventPayload::Failure { message }
                        if message.contains("failed to bind")
                    )
                }));
            }
            other => panic!("unexpected serve result: {other:?}"),
        }
    }
}
