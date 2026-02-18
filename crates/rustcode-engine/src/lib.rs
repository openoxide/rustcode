use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::{
    env,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;
use globset::Glob;
use regex::Regex;

use rustcode_core::command::{AgentOptions, Command};
use rustcode_core::context::CommandContext;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::permissions::PermissionAction;
use rustcode_core::ports::{
    CommandExecutor, EventPublisher, PathOperation, PermissionPolicy, ToolApprover,
    TranscriptRecorder,
};
use rustcode_core::session::{MessageRole, StoredMessage, StoredToolCall};
use rustcode_core::ToolApprovalRequest;
use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
use rustcode_llm::{
    ChatMessage, ChatRequest, ChatRole, LlmClient, LlmRequest, RequestInitiator, ToolCall,
};
use rustcode_plugins::PluginRegistry;

mod agent_tools;

const WEBFETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WEBFETCH_DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const WEBFETCH_MAX_TIMEOUT: Duration = Duration::from_secs(120);
const WEBFETCH_MAX_BODY_BYTES: usize = 1_000_000;

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
    recorder: Option<Arc<dyn TranscriptRecorder>>,
    approver: Option<Arc<dyn ToolApprover>>,
    next_event_id: AtomicU64,
    next_message_id: AtomicU64,
}

#[derive(Debug, Default)]
pub(crate) struct AgentState {
    read_paths: HashSet<PathBuf>,
}

impl Engine {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        fs: Arc<dyn FileSystemPort>,
        process: Arc<dyn ProcessPort>,
        permission_policy: Arc<dyn PermissionPolicy>,
        plugins: PluginRegistry,
        recorder: Option<Arc<dyn TranscriptRecorder>>,
        approver: Option<Arc<dyn ToolApprover>>,
    ) -> Self {
        Self {
            llm,
            fs,
            process,
            permission_policy,
            plugins,
            recorder,
            approver,
            next_event_id: AtomicU64::new(1),
            next_message_id: AtomicU64::new(1),
        }
    }

    fn new_message_id(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = self.next_message_id.fetch_add(1, Ordering::Relaxed);
        format!("m-{now}-{seq}")
    }

    async fn record_message(
        &self,
        context: &CommandContext,
        message: StoredMessage,
    ) -> Result<(), ExecutionError> {
        let Some(recorder) = self.recorder.as_ref() else {
            return Ok(());
        };
        recorder
            .append_message(&context.session.session_id, message)
            .await
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
        let user_message_id = self.new_message_id();
        self.record_message(
            context,
            StoredMessage {
                id: user_message_id,
                role: MessageRole::User,
                created_at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                content: Value::String(prompt.clone()),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
        )
        .await?;

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
            let text = response.text;
            self.emit(
                publisher,
                EventScope::Command,
                EventPayload::OutputChunk {
                    text: text.clone(),
                },
                context,
            )
            .await?;

            let assistant_message_id = self.new_message_id();
            self.record_message(
                context,
                StoredMessage {
                    id: assistant_message_id,
                    role: MessageRole::Assistant,
                    created_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                    content: Value::String(text),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                },
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

        let assistant_message_id = self.new_message_id();
        self.record_message(
            context,
            StoredMessage {
                id: assistant_message_id,
                role: MessageRole::Assistant,
                created_at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                content: Value::String(response.text),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
        )
        .await?;

        Ok(())
    }

    async fn run_agent(
        &self,
        prompt: String,
        options: AgentOptions,
        history: Vec<StoredMessage>,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let tools =
            agent_tools::AgentToolRegistry::tool_specs(&options, context.config.allow_network);
        let mut state = AgentState::default();

        let system_prompt = "You are rustcode, a production-grade coding agent.\n\
Use tools when you need filesystem context.\n\
Prefer: list -> read.\n\
Only modify files via write/edit when explicitly required.\n\
When you are done, respond with a final plain-text answer."
            .to_string();

        let mut messages = Vec::new();
        if history.is_empty() {
            let system_id = self.new_message_id();
            self.record_message(
                context,
                StoredMessage {
                    id: system_id,
                    role: MessageRole::System,
                    created_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                    content: Value::String(system_prompt.clone()),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                },
            )
            .await?;
            messages.push(ChatMessage {
                role: ChatRole::System,
                content: Value::String(system_prompt),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            });
        } else {
            messages.extend(stored_messages_to_chat(history));
            if !matches!(messages.first().map(|m| m.role), Some(ChatRole::System)) {
                messages.insert(
                    0,
                    ChatMessage {
                        role: ChatRole::System,
                        content: Value::String(system_prompt),
                        tool_call_id: None,
                        tool_name: None,
                        tool_calls: Vec::new(),
                    },
                );
            }
        }

        let user_id = self.new_message_id();
        self.record_message(
            context,
            StoredMessage {
                id: user_id.clone(),
                role: MessageRole::User,
                created_at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                content: Value::String(prompt.clone()),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
        )
        .await?;

        messages.push(ChatMessage {
            role: ChatRole::User,
            content: Value::String(prompt),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        });

        for _step in 0..options.max_steps {
            let request = ChatRequest {
                model: context.config.model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                initiator: RequestInitiator::Agent,
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
                tool_name: None,
                tool_calls: response.tool_calls.clone(),
            });

            let assistant_id = self.new_message_id();
            self.record_message(
                context,
                StoredMessage {
                    id: assistant_id,
                    role: MessageRole::Assistant,
                    created_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                    content: if response.text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(response.text.clone())
                    },
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: response
                        .tool_calls
                        .iter()
                        .map(|call| StoredToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        })
                        .collect(),
                },
            )
            .await?;

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

            for call in response
                .tool_calls
                .iter()
                .take(options.max_tool_calls_per_step)
            {
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
                    .execute_agent_tool_call(
                        call.name.as_str(),
                        call.arguments.as_str(),
                        context,
                        &options,
                        &mut state,
                    )
                    .await;

                let (ok, output) = match tool_result {
                    Ok(output) => (true, output),
                    Err(ExecutionError::Dispatch(message)) => (false, message),
                    Err(ExecutionError::Executor(message)) => (false, message),
                    Err(err) => (false, err.to_string()),
                };
                let result_payload = tool_payload_json(ok, output, options.max_tool_result_bytes);

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

                let tool_msg_id = self.new_message_id();
                self.record_message(
                    context,
                    StoredMessage {
                        id: tool_msg_id,
                        role: MessageRole::Tool,
                        created_at_unix_ms: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                        content: Value::String(result_payload.clone()),
                        tool_call_id: Some(call.id.clone()),
                        tool_name: Some(call.name.clone()),
                        tool_calls: Vec::new(),
                    },
                )
                .await?;

                messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: Value::String(result_payload),
                    tool_call_id: Some(call.id.clone()),
                    tool_name: Some(call.name.clone()),
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
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        if context.cancellation.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        let args: Value = serde_json::from_str(arguments).map_err(|err| {
            ExecutionError::Dispatch(format!("tool arguments are not valid JSON: {err}"))
        })?;

        if is_mutating_tool(name) {
            let (permission, pattern, reason) = approval_fields(name, &args);
            let match_targets = approval_match_targets(name, &args);
            let decision = resolve_permission_action(
                &context.config.permission_rules,
                &permission,
                &match_targets,
            )?;

            match decision {
                Some(PermissionAction::Deny) => {
                    return Err(ExecutionError::Dispatch(format!(
                        "tool permission denied by rule: tool={name} permission={permission} target={pattern}"
                    )));
                }
                Some(PermissionAction::Allow) => {
                    // Explicit allow: proceed without prompting.
                }
                Some(PermissionAction::Ask) | None => {
                    let Some(approver) = self.approver.as_ref() else {
                        return Err(ExecutionError::Dispatch(format!(
                            "tool approval required but no interactive approver is available: tool={name} permission={permission} target={pattern}"
                        )));
                    };
                    let approved = approver
                        .approve(ToolApprovalRequest {
                            tool: name.to_string(),
                            permission,
                            pattern,
                            arguments: args.clone(),
                            reason,
                        })
                        .await?;
                    if !approved {
                        return Err(ExecutionError::Dispatch(
                            "tool execution rejected by user".to_string(),
                        ));
                    }
                }
            }
        }

        agent_tools::AgentToolRegistry::execute(self, name, args, context, options, state).await
    }

    async fn agent_tool_exec(
        &self,
        command: &str,
        args: &[String],
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let output = self
            .process
            .run_capture(
                command,
                args,
                &context.config.workspace_root,
                context.cancellation.clone(),
            )
            .await
            .map_err(|err| match err {
                IoError::Cancelled => ExecutionError::Cancelled,
                _ => ExecutionError::Executor(err.to_string()),
            })?;

        let mut rendered = String::new();
        rendered.push_str("exit_code=");
        rendered.push_str(&output.code.to_string());
        rendered.push('\n');
        if !output.stdout.is_empty() {
            rendered.push_str("stdout:\n");
            rendered.push_str(&output.stdout);
            if !output.stdout.ends_with('\n') {
                rendered.push('\n');
            }
        }
        if !output.stderr.is_empty() {
            rendered.push_str("stderr:\n");
            rendered.push_str(&output.stderr);
            if !output.stderr.ends_with('\n') {
                rendered.push('\n');
            }
        }

        if output.code == 0 {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }

    async fn agent_tool_webfetch(
        &self,
        url: &str,
        format: Option<&str>,
        timeout_secs: Option<u64>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if !context.config.allow_network {
            return Err(ExecutionError::Dispatch(
                "network access is disabled; set allow_network=true (or RUSTCODE_ALLOW_NETWORK=1) to enable webfetch"
                    .to_string(),
            ));
        }

        let parsed = reqwest::Url::parse(url).map_err(|err| {
            ExecutionError::Dispatch(format!("webfetch url is invalid: {err}"))
        })?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(ExecutionError::Dispatch(format!(
                    "webfetch url must use http or https (got scheme={other})"
                )));
            }
        }

        let timeout = timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(WEBFETCH_DEFAULT_TIMEOUT)
            .clamp(Duration::from_secs(1), WEBFETCH_MAX_TIMEOUT);

        let client = reqwest::Client::builder()
            .connect_timeout(WEBFETCH_CONNECT_TIMEOUT)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .no_proxy()
            .build()
            .map_err(|err| {
                ExecutionError::Executor(format!("failed to build webfetch http client: {err}"))
            })?;

        let response = tokio::select! {
            _ = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            result = client.get(parsed.clone()).send() => {
                result.map_err(|err| ExecutionError::Executor(format!("webfetch request failed: {err}")))?
            }
        };

        let status = response.status();
        let final_url = response.url().clone();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        let is_html = content_type.to_ascii_lowercase().contains("text/html");
        let output_format = format.unwrap_or("markdown").trim().to_ascii_lowercase();

        let mut stream = response.bytes_stream();
        let mut body: Vec<u8> = Vec::new();
        let mut body_truncated = false;
        while let Some(chunk) = tokio::select! {
            _ = context.cancellation.cancelled() => {
                return Err(ExecutionError::Cancelled);
            }
            next = stream.next() => next
        } {
            let chunk = chunk.map_err(|err| {
                ExecutionError::Executor(format!("webfetch response stream failed: {err}"))
            })?;
            if body.len().saturating_add(chunk.len()) > WEBFETCH_MAX_BODY_BYTES {
                let remaining = WEBFETCH_MAX_BODY_BYTES.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
                body_truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let raw = String::from_utf8_lossy(&body).to_string();
        let content = if is_html && output_format != "html" {
            html_to_plainish_text(&raw)
        } else {
            raw
        };

        let mut rendered = String::new();
        rendered.push_str("status=");
        rendered.push_str(status.as_str());
        rendered.push('\n');
        rendered.push_str("final_url=");
        rendered.push_str(final_url.as_str());
        rendered.push('\n');
        if !content_type.is_empty() {
            rendered.push_str("content_type=");
            rendered.push_str(&content_type);
            rendered.push('\n');
        }
        rendered.push_str("body_truncated=");
        rendered.push_str(if body_truncated { "true" } else { "false" });
        rendered.push('\n');
        rendered.push_str("content:\n");
        rendered.push_str(&content);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }

        if status.is_success() {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }

    async fn agent_tool_list(
        &self,
        path: Option<String>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let target = path.unwrap_or_else(|| ".".to_string());
        let resolved = self.resolve_workspace_path(context, &target, PathOperation::List)?;
        let entries = self
            .fs
            .list_dir_limited(&resolved, options.max_list_entries)
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
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Read)?;
        state.read_paths.insert(resolved.clone());
        self.fs
            .read_to_string_limited(&resolved, options.max_read_bytes)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))
    }

    async fn agent_tool_write(
        &self,
        path: &str,
        contents: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Write)?;
        if !state.read_paths.contains(&resolved)
            && self
                .fs
                .exists(&resolved)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?
        {
            return Err(ExecutionError::Dispatch(
                "write requires reading the target file first (refusing to overwrite unread file)"
                    .to_string(),
            ));
        }
        if contents.len() > options.max_write_bytes {
            return Err(ExecutionError::Dispatch(format!(
                "write contents exceeds max-write-bytes limit ({} > {})",
                contents.len(),
                options.max_write_bytes
            )));
        }
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
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Edit)?;
        if !state.read_paths.contains(&resolved) {
            return Err(ExecutionError::Dispatch(
                "edit requires reading the target file first".to_string(),
            ));
        }
        let original = self
            .fs
            .read_to_string_limited(&resolved, options.max_read_bytes)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        if original.contains("[rustcode:truncated]") {
            return Err(ExecutionError::Dispatch(
                "refusing to edit a truncated read; increase --max-read-bytes".to_string(),
            ));
        }

        let replacements = original.matches(from).count();
        let updated = original.replace(from, to);
        self.fs
            .write_string(&resolved, &updated)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        Ok(format!(
            "edit applied ({replacements} replacements) to {}",
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

    async fn agent_tool_glob(
        &self,
        pattern: &str,
        root: Option<&str>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let root = root.unwrap_or(".");
        let resolved_root = self.resolve_workspace_path(context, root, PathOperation::List)?;
        let matcher = Glob::new(pattern)
            .map_err(|err| ExecutionError::Dispatch(format!("invalid glob pattern: {err}")))?
            .compile_matcher();
        let workspace_root = absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let candidates = self
            .fs
            .walk_dir_limited(&resolved_root, options.max_list_entries)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        let mut count = 0usize;
        for path in candidates {
            let rel_for_match = path
                .strip_prefix(&resolved_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string()
                .replace('\\', "/");
            if !matcher.is_match(&rel_for_match) {
                continue;
            }

            let relative = path
                .strip_prefix(&workspace_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            rendered.push_str(&relative);
            rendered.push('\n');
            count += 1;
            if count >= options.max_list_entries {
                break;
            }
        }

        Ok(rendered)
    }

    async fn agent_tool_grep(
        &self,
        pattern: &str,
        root: Option<&str>,
        include: Option<&str>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let root = root.unwrap_or(".");
        let resolved_root = self.resolve_workspace_path(context, root, PathOperation::List)?;
        let workspace_root = absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let re = Regex::new(pattern)
            .map_err(|err| ExecutionError::Dispatch(format!("invalid regex pattern: {err}")))?;
        let include_matcher = if let Some(include) = include {
            Some(
                Glob::new(include)
                    .map_err(|err| ExecutionError::Dispatch(format!("invalid include glob: {err}")))?
                    .compile_matcher(),
            )
        } else {
            None
        };

        let candidates = self
            .fs
            .walk_dir_limited(&resolved_root, options.max_list_entries)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        let mut matches = 0usize;
        for path in candidates {
            let meta = self
                .fs
                .metadata(&path)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?;
            if !meta.is_file {
                continue;
            }

            let relative = path
                .strip_prefix(&workspace_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            if let Some(include) = include_matcher.as_ref() {
                let normalized = relative.replace('\\', "/");
                if !include.is_match(&normalized) {
                    continue;
                }
            }

            let content = self
                .fs
                .read_to_string_limited(&path, options.max_read_bytes)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?;

            for (idx, line) in content.lines().enumerate() {
                if !re.is_match(line) {
                    continue;
                }
                let line_no = idx + 1;
                rendered.push_str(&relative);
                rendered.push(':');
                rendered.push_str(&line_no.to_string());
                rendered.push_str(": ");
                rendered.push_str(line);
                rendered.push('\n');
                matches += 1;
                if matches >= 2000 {
                    return Ok(rendered);
                }
            }
        }

        Ok(rendered)
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
            Command::Agent {
                prompt,
                options,
                history,
            } => {
                self.run_agent(prompt, options, history, &context, publisher.clone())
                    .await
            }
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

// Tool specs are defined in `agent_tools::AgentToolRegistry`.

fn stored_messages_to_chat(history: Vec<StoredMessage>) -> Vec<ChatMessage> {
    history
        .into_iter()
        .map(|msg| ChatMessage {
            role: match msg.role {
                MessageRole::System => ChatRole::System,
                MessageRole::User => ChatRole::User,
                MessageRole::Assistant => ChatRole::Assistant,
                MessageRole::Tool => ChatRole::Tool,
            },
            content: msg.content,
            tool_call_id: msg.tool_call_id,
            tool_name: msg.tool_name,
            tool_calls: msg
                .tool_calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                })
                .collect(),
        })
        .collect()
}

fn tool_payload_json(ok: bool, output: String, max_bytes: usize) -> String {
    let mut truncated = false;
    let mut candidate_output = output;

    // Prefer preserving structured JSON while bounding the size deterministically.
    // If output is too large, progressively truncate the output string until the JSON fits.
    for _ in 0..8 {
        let payload = json!({
            "ok": ok,
            "truncated": truncated,
            "output": candidate_output,
        })
        .to_string();

        if payload.len() <= max_bytes {
            return payload;
        }

        truncated = true;
        let limit = max_bytes.saturating_sub(256);
        candidate_output = truncate_utf8_bytes(&candidate_output, limit);
    }

    json!({
        "ok": ok,
        "truncated": true,
        "output": "[tool output truncated]".to_string(),
    })
    .to_string()
}

fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = 0usize;
    for (idx, _) in input.char_indices() {
        if idx > max_bytes {
            break;
        }
        end = idx;
    }
    let prefix = &input[..end];
    format!("{prefix}\n...[truncated]...\n")
}

fn resolve_permission_action(
    rules: &[rustcode_core::PermissionRule],
    permission: &str,
    targets: &[String],
) -> Result<Option<PermissionAction>, ExecutionError> {
    let normalized_targets = targets
        .iter()
        .map(|target| target.replace('\\', "/"))
        .collect::<Vec<_>>();

    for rule in rules.iter().rev() {
        if !rule.permission.eq_ignore_ascii_case(permission) {
            continue;
        }
        let matcher = Glob::new(&rule.pattern)
            .map_err(|err| {
                ExecutionError::Dispatch(format!(
                    "invalid permissions glob pattern {}: {err}",
                    rule.pattern
                ))
            })?
            .compile_matcher();

        if normalized_targets
            .iter()
            .any(|target| !target.is_empty() && matcher.is_match(target))
        {
            return Ok(Some(rule.action));
        }
    }

    Ok(None)
}

fn html_to_plainish_text(html: &str) -> String {
    // Intentionally lightweight: no additional deps, deterministic output.
    // Removes scripts/styles, inserts line breaks for common block tags, strips remaining tags,
    // decodes a handful of common entities, and normalizes whitespace.
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let re_br = Regex::new(r"(?is)<br\s*/?>").unwrap();
    let re_block_end = Regex::new(r"(?is)</\s*(p|div|li|h[1-6]|tr|table|ul|ol)\s*>").unwrap();
    let re_li = Regex::new(r"(?is)<\s*li\b[^>]*>").unwrap();
    let re_tags = Regex::new(r"(?is)<[^>]+>").unwrap();
    let re_ws = Regex::new(r"[ \t\x0B\x0C\r]+\n").unwrap();
    let re_many_newlines = Regex::new(r"\n{3,}").unwrap();

    let mut s = re_script.replace_all(html, "").to_string();
    s = re_style.replace_all(&s, "").to_string();
    s = re_br.replace_all(&s, "\n").to_string();
    s = re_block_end.replace_all(&s, "\n").to_string();
    s = re_li.replace_all(&s, "- ").to_string();
    s = re_tags.replace_all(&s, "").to_string();

    s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");

    s = re_ws.replace_all(&s, "\n").to_string();
    s = re_many_newlines.replace_all(&s, "\n\n").to_string();

    s.trim().to_string()
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write" | "edit" | "exec")
}

fn approval_fields(tool: &str, args: &Value) -> (String, String, String) {
    let permission = tool.to_string();
    let (pattern, reason) = match tool {
        "write" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to write {path}"),
            )
        }
        "edit" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to edit {path}"),
            )
        }
        "exec" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let mut rendered = command.to_string();
            if let Some(items) = args.get("args").and_then(Value::as_array) {
                for item in items.iter().filter_map(Value::as_str) {
                    rendered.push(' ');
                    rendered.push_str(item);
                }
            }
            (
                rendered.clone(),
                format!("agent requests permission to execute {rendered}"),
            )
        }
        _ => (
            "*".to_string(),
            "agent requests permission for tool execution".to_string(),
        ),
    };

    (permission, pattern, reason)
}

fn approval_match_targets(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        "exec" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut targets = Vec::new();
            if !command.is_empty() {
                targets.push(command.clone());
            }
            if let Some(items) = args.get("args").and_then(Value::as_array) {
                let mut rendered = command.clone();
                for item in items.iter().filter_map(Value::as_str) {
                    if !rendered.is_empty() {
                        rendered.push(' ');
                    }
                    rendered.push_str(item);
                }
                if !rendered.is_empty() && rendered != command {
                    targets.push(rendered);
                }
            }
            if targets.is_empty() {
                targets.push(String::new());
            }
            targets
        }
        "write" | "edit" => vec![args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        _ => vec!["*".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use async_trait::async_trait;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tokio::time::timeout;
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
    struct StubProcess {
        stdout: String,
        stderr: String,
        code: i32,
    }
    struct CountingApprover {
        approved: bool,
        calls: AtomicUsize,
    }
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

        async fn read_to_string_limited(
            &self,
            _path: &Path,
            _max_bytes: usize,
        ) -> Result<String, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn exists(&self, _path: &Path) -> Result<bool, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn list_dir_limited(
            &self,
            _path: &Path,
            _max_entries: usize,
        ) -> Result<Vec<PathBuf>, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn metadata(&self, _path: &Path) -> Result<rustcode_io::FsMetadata, IoError> {
            Err(IoError::Io("not used".to_string()))
        }

        async fn walk_dir_limited(
            &self,
            _path: &Path,
            _max_entries: usize,
        ) -> Result<Vec<PathBuf>, IoError> {
            Err(IoError::Io("not used".to_string()))
        }
    }

    #[async_trait]
    impl FileSystemPort for AgentFs {
        async fn read_to_string(&self, _path: &Path) -> Result<String, IoError> {
            Ok("agent-read-ok".to_string())
        }

        async fn read_to_string_limited(
            &self,
            _path: &Path,
            _max_bytes: usize,
        ) -> Result<String, IoError> {
            Ok("agent-read-ok".to_string())
        }

        async fn exists(&self, _path: &Path) -> Result<bool, IoError> {
            Ok(true)
        }

        async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
            Ok(())
        }

        async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
            Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
        }

        async fn list_dir_limited(
            &self,
            _path: &Path,
            _max_entries: usize,
        ) -> Result<Vec<PathBuf>, IoError> {
            Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
        }

        async fn metadata(&self, path: &Path) -> Result<rustcode_io::FsMetadata, IoError> {
            let is_dir = path.ends_with("dir");
            Ok(rustcode_io::FsMetadata {
                is_dir,
                is_file: !is_dir,
                len: 0,
            })
        }

        async fn walk_dir_limited(
            &self,
            _path: &Path,
            _max_entries: usize,
        ) -> Result<Vec<PathBuf>, IoError> {
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

        async fn run_capture(
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
    impl ProcessPort for StubProcess {
        async fn run(
            &self,
            _program: &str,
            _args: &[String],
            _cwd: &Path,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, IoError> {
            if self.code == 0 {
                Ok(ProcessOutput {
                    code: 0,
                    stdout: self.stdout.clone(),
                    stderr: self.stderr.clone(),
                })
            } else {
                Err(IoError::Exit {
                    code: self.code,
                    stderr: self.stderr.clone(),
                })
            }
        }

        async fn run_capture(
            &self,
            _program: &str,
            _args: &[String],
            _cwd: &Path,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, IoError> {
            Ok(ProcessOutput {
                code: self.code,
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    #[async_trait]
    impl ToolApprover for CountingApprover {
        async fn approve(&self, _request: ToolApprovalRequest) -> Result<bool, ExecutionError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.approved)
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
            None,
            None,
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
            None,
            None,
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
                    options: AgentOptions::default(),
                    history: Vec::new(),
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
    async fn agent_can_exec_when_allowed() {
        struct ExecAgentLlm {
            step: Mutex<usize>,
        }

        #[async_trait]
        impl LlmClient for ExecAgentLlm {
            async fn complete(
                &self,
                _request: LlmRequest,
            ) -> Result<LlmResponse, rustcode_llm::LlmError> {
                Ok(LlmResponse {
                    text: "unused".to_string(),
                    chunks: Vec::new(),
                })
            }

            async fn chat(
                &self,
                request: ChatRequest,
            ) -> Result<ChatResponse, rustcode_llm::LlmError> {
                let mut step = self.step.lock().await;
                match *step {
                    0 => {
                        *step = 1;
                        assert!(
                            request.tools.iter().any(|tool| tool.name == "exec"),
                            "expected exec tool spec when allow-exec is enabled"
                        );
                        Ok(ChatResponse {
                            text: String::new(),
                            tool_calls: vec![ToolCall {
                                id: "call_exec".to_string(),
                                name: "exec".to_string(),
                                arguments: r#"{"command":"echo","args":["hi"]}"#.to_string(),
                            }],
                        })
                    }
                    _ => {
                        assert!(
                            request
                                .messages
                                .iter()
                                .any(|msg| msg.tool_call_id.as_deref() == Some("call_exec")),
                            "expected tool result message for call_exec"
                        );
                        Ok(ChatResponse {
                            text: "done".to_string(),
                            tool_calls: Vec::new(),
                        })
                    }
                }
            }
        }

        let workspace_root = PathBuf::from("/tmp/rustcode-agent-exec-workspace");
        let engine = Engine::new(
            Arc::new(ExecAgentLlm {
                step: Mutex::new(0),
            }),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hi\n".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );
        let publisher = Arc::new(CollectingPublisher::default());
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                workspace_root,
                permission_rules: vec![rustcode_core::PermissionRule {
                    permission: "exec".to_string(),
                    action: PermissionAction::Allow,
                    pattern: "echo".to_string(),
                }],
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "agent-exec-s1".to_string(),
                request_id: "agent-exec-r1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let result = engine
            .execute(
                Command::Agent {
                    prompt: "hi".to_string(),
                    options: AgentOptions {
                        allow_exec: true,
                        ..AgentOptions::default()
                    },
                    history: Vec::new(),
                },
                context,
                publisher.clone(),
            )
            .await;
        assert!(result.is_ok(), "result={result:?}");

        let events = publisher.events.lock().await.clone();
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::ToolCall { name, .. } if name == "exec"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::ToolResult { name, ok, output, .. }
                if name == "exec" && *ok && output.contains("exit_code=0")
            )
        }));
    }

    #[tokio::test]
    async fn read_rejects_workspace_escape() {
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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
            None,
            None,
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

    #[tokio::test]
    async fn mutating_tools_require_approval_when_no_allow_rule_and_no_approver() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules.clear();
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-1".to_string(),
                request_id: "r-approve-1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "ok".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 1024,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let result = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo","args":["hi"]}"#,
                &context,
                &options,
                &mut state,
            )
            .await;
        match result {
            Err(ExecutionError::Dispatch(message)) => {
                assert!(message.contains("approval required"), "message={message}");
            }
            other => panic!("expected dispatch error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mutating_tools_can_be_allowed_by_permissions_without_prompt() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules = vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Allow,
            pattern: "echo".to_string(),
        }];
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-2".to_string(),
                request_id: "r-approve-2".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hello".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 4096,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let output = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo","args":["hi"]}"#,
                &context,
                &options,
                &mut state,
            )
            .await
            .expect("exec should be allowed");
        assert!(output.contains("exit_code=0"), "output={output}");
    }

    #[tokio::test]
    async fn exec_permission_rule_can_match_full_command_line() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules = vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Allow,
            pattern: "echo hi".to_string(),
        }];
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-exec-full-1".to_string(),
                request_id: "r-approve-exec-full-1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hello".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 4096,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let output = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo","args":["hi"]}"#,
                &context,
                &options,
                &mut state,
            )
            .await
            .expect("exec should be allowed");
        assert!(output.contains("exit_code=0"), "output={output}");
    }

    #[tokio::test]
    async fn exec_permission_rule_precedence_prefers_last_match_across_targets() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules = vec![
            rustcode_core::PermissionRule {
                permission: "exec".to_string(),
                action: PermissionAction::Allow,
                pattern: "echo".to_string(),
            },
            rustcode_core::PermissionRule {
                permission: "exec".to_string(),
                action: PermissionAction::Deny,
                pattern: "echo hi".to_string(),
            },
        ];
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-exec-full-2".to_string(),
                request_id: "r-approve-exec-full-2".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let approver = Arc::new(CountingApprover {
            approved: true,
            calls: AtomicUsize::new(0),
        });
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hello".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            Some(approver.clone()),
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 1024,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let result = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo","args":["hi"]}"#,
                &context,
                &options,
                &mut state,
            )
            .await;
        assert!(matches!(result, Err(ExecutionError::Dispatch(_))));
        assert_eq!(approver.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn mutating_tools_can_be_denied_by_permissions_without_prompt() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules = vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Deny,
            pattern: "echo".to_string(),
        }];
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-3".to_string(),
                request_id: "r-approve-3".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let approver = Arc::new(CountingApprover {
            approved: true,
            calls: AtomicUsize::new(0),
        });
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hello".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            Some(approver.clone()),
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 1024,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let result = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo"}"#,
                &context,
                &options,
                &mut state,
            )
            .await;
        assert!(matches!(result, Err(ExecutionError::Dispatch(_))));
        assert_eq!(approver.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn mutating_tools_ask_rule_triggers_approver() {
        let mut cfg = ResolvedConfig::default();
        cfg.permission_rules = vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Ask,
            pattern: "echo".to_string(),
        }];
        let context = CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: "s-approve-4".to_string(),
                request_id: "r-approve-4".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let approver = Arc::new(CountingApprover {
            approved: true,
            calls: AtomicUsize::new(0),
        });
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(StubProcess {
                stdout: "hello".to_string(),
                stderr: String::new(),
                code: 0,
            }),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            Some(approver.clone()),
        );

        let options = AgentOptions {
            max_steps: 1,
            max_tool_calls_per_step: 1,
            allow_write: false,
            allow_edit: false,
            allow_exec: true,
            max_read_bytes: 1024,
            max_list_entries: 100,
            max_tool_result_bytes: 4096,
            max_write_bytes: 1024,
        };
        let mut state = AgentState::default();
        let output = engine
            .execute_agent_tool_call(
                "exec",
                r#"{"command":"echo"}"#,
                &context,
                &options,
                &mut state,
            )
            .await
            .expect("exec should be approved");
        assert!(output.contains("exit_code=0"), "output={output}");
        assert_eq!(approver.calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn webfetch_rejects_when_network_disabled() {
        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                allow_network: false,
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "s-webfetch-1".to_string(),
                request_id: "r-webfetch-1".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let options = AgentOptions::default();
        let mut state = AgentState::default();
        let result = engine
            .execute_agent_tool_call(
                "webfetch",
                r#"{"url":"http://127.0.0.1/"}"#,
                &context,
                &options,
                &mut state,
            )
            .await;
        match result {
            Err(ExecutionError::Dispatch(message)) => {
                assert!(message.contains("network access is disabled"), "message={message}");
            }
            other => panic!("expected dispatch error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn webfetch_fetches_local_http_and_simplifies_html_by_default() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener must bind");
        let addr = listener.local_addr().expect("listener addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = timeout(Duration::from_secs(1), socket.read(&mut buf)).await;

            let body = "<html><body><h1>Hello</h1><p>World</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                allow_network: true,
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "s-webfetch-2".to_string(),
                request_id: "r-webfetch-2".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let url = format!("http://{}/", addr);
        let options = AgentOptions::default();
        let mut state = AgentState::default();
        let output = engine
            .execute_agent_tool_call(
                "webfetch",
                &format!(r#"{{"url":"{url}"}}"#),
                &context,
                &options,
                &mut state,
            )
            .await
            .expect("webfetch should succeed");

        server.await.expect("server must join");

        assert!(output.contains("status=200"), "output={output}");
        assert!(output.contains("Hello"), "output={output}");
        assert!(output.contains("World"), "output={output}");
        assert!(!output.contains("<html>"), "output={output}");
    }

    #[tokio::test]
    async fn webfetch_format_html_returns_raw_html() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener must bind");
        let addr = listener.local_addr().expect("listener addr");

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = timeout(Duration::from_secs(1), socket.read(&mut buf)).await;

            let body = "<html><body>ok</body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let engine = Engine::new(
            Arc::new(NullLlmClient),
            Arc::new(DummyFs),
            Arc::new(CancelledProcess),
            Arc::new(WorkspacePermissionPolicy),
            PluginRegistry::default(),
            None,
            None,
        );
        let context = CommandContext::new(
            Arc::new(ResolvedConfig {
                allow_network: true,
                ..ResolvedConfig::default()
            }),
            SessionMeta {
                session_id: "s-webfetch-3".to_string(),
                request_id: "r-webfetch-3".to_string(),
                started_at: SystemTime::now(),
            },
        );

        let url = format!("http://{}/", addr);
        let options = AgentOptions::default();
        let mut state = AgentState::default();
        let output = engine
            .execute_agent_tool_call(
                "webfetch",
                &format!(r#"{{"url":"{url}","format":"html"}}"#),
                &context,
                &options,
                &mut state,
            )
            .await
            .expect("webfetch should succeed");

        server.await.expect("server must join");

        assert!(output.contains("<html>"), "output={output}");
    }
}
