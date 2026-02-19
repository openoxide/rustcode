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
use futures_util::future::join_all;
use futures_util::StreamExt;
use globset::Glob;
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use rustcode_core::command::{AgentOptions, Command};
use rustcode_core::context::CommandContext;
use rustcode_core::error::{ExecutionError, PublishError};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::permissions::PermissionAction;
use rustcode_core::ports::{
    CommandExecutor, EventPublisher, PathOperation, PermissionPolicy, ToolApprover,
    TranscriptRecorder,
};
use rustcode_core::server_protocol::{
    V1ErrorResponse, V1RunRequest, V1SessionCreateRequest, V1SessionCreateResponse,
    V1SessionShowResponse, V1SessionsListResponse, SERVER_API_SCHEMA_VERSION,
};
use rustcode_core::session::{MessageRole, StoredMessage, StoredToolCall};
use rustcode_core::ToolApprovalRequest;
use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
use rustcode_llm::{
    ChatMessage, ChatRequest, ChatRole, LlmClient, LlmRequest, RequestInitiator, ToolCall,
};
use rustcode_plugins::PluginRegistry;
use rustcode_state::SessionStore;

mod agent_handlers_bash;
mod agent_handlers_fs;
mod agent_handlers_interactive;
mod agent_handlers_lsp;
mod agent_handlers_multiedit;
mod agent_handlers_patch;
mod agent_handlers_pty;
mod agent_handlers_search;
mod agent_handlers_web;
mod agent_handlers_worktree;
mod agent_runtime;
mod agent_tool_specs;
mod agent_tools;
mod agent_util;
pub mod compaction;
pub mod context_tracker;
mod engine_commands;
pub mod instructions;
pub mod mcp;
mod path_utils;
pub mod retry;
pub mod scheduler;
mod serve_api;
mod serve_http;
mod serve_runtime;
pub mod session_summary;
pub mod system_prompt;

#[derive(Clone)]
pub struct ChannelPublisher {
    sender: mpsc::Sender<Event>,
}

impl ChannelPublisher {
    #[must_use]
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
    mcp: Option<mcp::McpRegistry>,
    /// Loaded skills for this workspace session.
    pub(crate) skills: rustcode_skills::SkillsManager,
    /// Memory storage (None if home directory is unavailable).
    pub(crate) memories: Option<Arc<rustcode_memories::MemoryStorage>>,
    /// Background scheduler handle (drives memory consolidation).
    scheduler: scheduler::SchedulerHandle,
    /// LSP manager for the workspace (None if no server detected / no workspace set).
    pub(crate) lsp_manager: Option<Arc<rustcode_lsp::LspManager>>,
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
            mcp: None,
            skills: rustcode_skills::SkillsManager::default(),
            memories: None,
            scheduler: scheduler::SchedulerHandle::default(),
            lsp_manager: None,
            next_event_id: AtomicU64::new(1),
            next_message_id: AtomicU64::new(1),
        }
    }

    /// Load skills from the given workspace root and start the background scheduler.
    ///
    /// Call this after constructing the engine to enable skill injection and memory consolidation.
    #[must_use]
    pub fn with_workspace(mut self, workspace_root: &Path) -> Self {
        self.skills = rustcode_skills::SkillsManager::load(workspace_root);
        let memories = rustcode_memories::MemoryStorage::new().map(Arc::new);
        let sched = if let Some(mem) = &memories {
            scheduler::SchedulerHandle::start(Some((
                mem.clone(),
                self.llm.clone(),
                String::new(), // model resolved per-request; consolidation uses current config
            )))
        } else {
            scheduler::SchedulerHandle::default()
        };
        self.memories = memories;
        self.scheduler = sched;
        self.lsp_manager = rustcode_lsp::LspManager::detect(workspace_root).map(Arc::new);
        self
    }

    /// Provide an explicit model name for the scheduler's memory consolidation task.
    ///
    /// Call after `with_workspace` if you have a specific model configured.
    #[must_use]
    pub fn with_memory_model(mut self, model: impl Into<String>) -> Self {
        // Restart scheduler with the real model name
        if let Some(mem) = &self.memories {
            self.scheduler = scheduler::SchedulerHandle::start(Some((
                mem.clone(),
                self.llm.clone(),
                model.into(),
            )));
        }
        self
    }

    #[must_use]
    pub fn with_mcp(mut self, mcp: mcp::McpRegistry) -> Self {
        self.mcp = Some(mcp);
        self
    }

    pub(crate) fn new_message_id(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = self.next_message_id.fetch_add(1, Ordering::Relaxed);
        format!("m-{now}-{seq}")
    }

    pub(crate) async fn record_message(
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

    pub(crate) async fn emit(
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
}

#[cfg(test)]
mod tests;
