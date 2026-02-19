use super::{
    ApprovalResponse, Arc, CancellationToken, InteractiveDefaults, InteractiveMsg,
    InteractiveSubmitMode, Line, MessageRole, Modifier, ResolvedConfig, SessionInfo, Size, Span,
    StoredMessage, Style, SystemTime, ToolApprovalRequest,
};
use std::time::Instant;

pub(super) enum Screen {
    Sessions,
    Chat(ChatState),
}

pub(super) struct PendingApproval {
    pub(super) request: ToolApprovalRequest,
    pub(super) reply: tokio::sync::oneshot::Sender<ApprovalResponse>,
}

pub(super) struct RunningCommand {
    pub(super) cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToastVariant {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub(super) struct Toast {
    pub(super) variant: ToastVariant,
    pub(super) message: String,
    pub(super) expires_at: SystemTime,
}

#[derive(Debug, Clone)]
pub(super) struct FindState {
    pub(super) query: String,
    pub(super) matches: Vec<usize>,
    pub(super) current: usize,
}

#[derive(Debug, Clone)]
pub(super) enum Modal {
    CommandPalette {
        query: String,
        selected: usize,
        items: Vec<CommandItem>,
        view: Vec<usize>,
    },
    Search {
        query: String,
        current: usize,
        matches: Vec<usize>,
    },
    FileSearch {
        query: String,
        entries: Vec<String>,
        view: Vec<usize>,
        selected: usize,
    },
    Rename {
        session_id: String,
        input: String,
        cursor: usize,
    },
    DeleteConfirm {
        session_id: String,
        title: String,
    },
    /// Full error detail popup (opened from footer error via Enter)
    ErrorDetail {
        message: String,
    },
    /// Skill toggle overlay — lists all loaded skills with toggle controls.
    SkillToggle {
        /// Snapshot of (name, description, enabled) for rendering and toggling.
        skills: Vec<(String, String, bool)>,
        /// Currently highlighted skill index.
        selected: usize,
    },
    /// Feedback overlay — thumbs-up/down rating + optional comment.
    Feedback {
        /// `true` = thumbs up, `false` = thumbs down, `None` = not yet selected.
        rating: Option<bool>,
        /// Optional comment text.
        comment: String,
        /// Whether keyboard focus is in the comment field.
        comment_active: bool,
    },
    /// Model picker — searchable list of all built-in models.
    ModelSelect {
        /// All model IDs formatted as `"provider/model-id"`, ordered by provider priority.
        entries: Vec<String>,
        /// Current search query for filtering.
        query: String,
        /// Filtered indices into `entries`.
        view: Vec<usize>,
        /// Selected index within `view`.
        selected: usize,
        /// Snapshot of the active model at the time the modal was opened.
        current_model: String,
    },
    /// Provider connection manager — connect/disconnect LLM providers.
    ProviderManager {
        step: ProviderManagerStep,
    },
}

/// An entry in the provider manager list.
#[derive(Debug, Clone)]
pub(super) struct ProviderEntry {
    pub(super) provider_id: String,
    pub(super) display_name: String,
    pub(super) connected: bool,
}

/// Auth method offered in the provider manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectMethod {
    ApiKey,
    OAuthDeviceCode,
    Disconnect,
}

/// Sent from OAuth background task when the device code is ready.
#[derive(Debug, Clone)]
pub(super) struct ProviderOAuthStarted {
    pub(super) provider_id: String,
    pub(super) verification_uri: String,
    pub(super) user_code: String,
}

/// Sent from OAuth background task when authentication completes.
#[derive(Debug, Clone)]
pub(super) struct ProviderOAuthDone {
    pub(super) provider_id: String,
    pub(super) access_token: String,
    pub(super) refresh_token: Option<String>,
    pub(super) expires_at_unix: Option<i64>,
    pub(super) account_id: Option<String>,
}

/// State machine for the provider manager modal.
#[derive(Debug, Clone)]
pub(super) enum ProviderManagerStep {
    /// Browse / search the provider list.
    List {
        entries: Vec<ProviderEntry>,
        query: String,
        view: Vec<usize>,
        selected: usize,
    },
    /// Choose how to connect the selected provider.
    MethodSelect {
        provider_id: String,
        display_name: String,
        methods: Vec<ConnectMethod>,
        selected: usize,
    },
    /// Type an API key.
    ApiKeyInput {
        provider_id: String,
        display_name: String,
        env_hint: Option<String>,
        input: String,
        cursor: usize,
    },
    /// Waiting for background task to start the device code flow.
    OAuthStarting {
        provider_id: String,
        display_name: String,
    },
    /// Device code ready — user must open browser and authorize.
    OAuthPending {
        provider_id: String,
        display_name: String,
        verification_uri: String,
        user_code: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommandId {
    Help,
    Sessions,
    NewSession,
    ForkSession,
    RenameSession,
    DeleteSession,
    Refresh,
    ToggleTools,
    Search,
    FileSearch,
    FocusComposer,
    FocusTranscript,
    FocusActivity,
    CancelRun,
    Quit,
    /// Open skill toggle overlay.
    ToggleSkills,
    /// Open feedback overlay.
    Feedback,
    /// Open model picker overlay.
    SwitchModel,
    /// Open provider connection manager overlay.
    ManageProviders,
}

#[derive(Debug, Clone)]
pub(super) struct CommandItem {
    pub(super) id: CommandId,
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) enabled: bool,
    pub(super) disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatFocus {
    Composer,
    Transcript,
    Activity,
}

#[derive(Debug, Clone)]
pub(super) enum ActivityItem {
    CommandAccepted {
        name: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        output: String,
    },
    OutputChunk {
        text: String,
    },
    Warning {
        message: String,
    },
    Failure {
        message: String,
    },
    Completed,
}

impl ActivityItem {
    pub(super) fn title(&self) -> String {
        match self {
            ActivityItem::CommandAccepted { name } => format!("command: {name}"),
            ActivityItem::ToolCall { name, .. } => format!("tool: {name}"),
            ActivityItem::ToolResult { name, ok, .. } => format!("tool result: {name} ok={ok}"),
            ActivityItem::OutputChunk { .. } => "assistant".to_string(),
            ActivityItem::Warning { .. } => "warning".to_string(),
            ActivityItem::Failure { .. } => "failure".to_string(),
            ActivityItem::Completed => "completed".to_string(),
        }
    }

    pub(super) fn summary(&self) -> String {
        let mut s = match self {
            ActivityItem::CommandAccepted { name } => name.clone(),
            ActivityItem::ToolCall { id, name, .. } => format!("{name} ({id})"),
            ActivityItem::ToolResult { id, name, ok, .. } => format!("{name} ({id}) ok={ok}"),
            ActivityItem::OutputChunk { text } => text.replace('\n', " "),
            ActivityItem::Warning { message } => message.clone(),
            ActivityItem::Failure { message } => message.clone(),
            ActivityItem::Completed => "done".to_string(),
        };
        if s.len() > 140 {
            s.truncate(140);
            s.push_str("...");
        }
        s
    }

    pub(super) fn details_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        out.push(Line::from(vec![Span::styled(
            self.title(),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        out.push(Line::raw(""));
        match self {
            ActivityItem::CommandAccepted { name } => {
                out.push(Line::raw(format!("name: {name}")));
            }
            ActivityItem::ToolCall {
                id,
                name,
                arguments,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(""));
                out.push(Line::raw("arguments:"));
                let pretty = serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| arguments.clone());
                for line in pretty.lines().take(32) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::ToolResult {
                id,
                name,
                ok,
                output,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(format!("ok: {ok}")));
                out.push(Line::raw(""));
                out.push(Line::raw("output:"));
                let pretty = serde_json::from_str::<serde_json::Value>(output)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| output.clone());
                for line in pretty.lines().take(48) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::OutputChunk { text } => {
                for line in text.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Warning { message } => {
                for line in message.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Failure { message } => {
                for line in message.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Completed => {}
        }

        out
    }
}

pub(super) struct ChatState {
    pub(super) session: SessionInfo,
    pub(super) messages: Vec<StoredMessage>,
    pub(super) scroll: u16,
    pub(super) live_assistant: String,
    pub(super) composer: String,
    pub(super) composer_cursor: usize,
    pub(super) prompt_history: Vec<String>,
    pub(super) history_cursor: Option<usize>,
    pub(super) history_draft: String,
    pub(super) focus: ChatFocus,
    pub(super) activity: Vec<ActivityItem>,
    pub(super) activity_selected: usize,
    pub(super) details_open: bool,
    pub(super) tool_details: bool,
    pub(super) find: Option<FindState>,
    pub(super) running: Option<RunningCommand>,
    /// The prompt most recently submitted but not yet confirmed by backend reload.
    pub(super) pending_prompt: Option<String>,
    /// Whether composer was just cleared by Ctrl+C (for "press again to exit" flow).
    pub(super) composer_cleared_by_ctrl_c: bool,
    /// Timestamp of last typing activity in composer.
    pub(super) last_typing_time: Option<Instant>,
}

pub(super) struct AppState {
    pub(super) sessions: Vec<SessionInfo>,
    pub(super) sessions_view: Vec<usize>,
    pub(super) selected: usize,
    pub(super) sessions_filter: String,
    pub(super) sessions_filter_active: bool,
    pub(super) screen: Screen,
    pub(super) status: Option<String>,
    pub(super) toasts: Vec<Toast>,
    pub(super) help_open: bool,
    pub(super) modal: Option<Modal>,
    pub(super) defaults: InteractiveDefaults,

    pub(super) pending_approval: Option<PendingApproval>,

    pub(super) submit_mode: InteractiveSubmitMode,

    pub(super) backend: Arc<dyn crate::SessionBackend>,
    pub(super) config: Option<Arc<ResolvedConfig>>,
    pub(super) executor: Option<Arc<dyn rustcode_core::ports::CommandExecutor>>,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) tx: tokio::sync::mpsc::UnboundedSender<InteractiveMsg>,
    pub(super) rx: tokio::sync::mpsc::UnboundedReceiver<InteractiveMsg>,
    pub(super) request_seq: u64,

    pub(super) last_area: Size,

    /// Receives `DeviceCodeFlowStart`-derived info once the OAuth device code is ready.
    /// Stored here (not in Modal) because Receiver is not Clone.
    pub(super) provider_oauth_start_rx:
        Option<std::sync::mpsc::Receiver<Result<ProviderOAuthStarted, String>>>,
    /// Receives the final credential once the user completes OAuth authorization.
    pub(super) provider_oauth_done_rx:
        Option<std::sync::mpsc::Receiver<Result<ProviderOAuthDone, String>>>,
    /// Shared cell that controls the engine's active LLM client.
    ///
    /// Writing a new `Arc<dyn LlmClient>` here causes all subsequent LLM
    /// requests to use the new client, enabling live model/provider switching
    /// without restarting the engine.  `None` in remote/attach mode.
    pub(super) llm_cell:
        Option<Arc<std::sync::RwLock<Arc<dyn rustcode_llm::LlmClient>>>>,
}

pub(super) fn build_prompt_history(messages: &[StoredMessage]) -> Vec<String> {
    let mut out = Vec::new();
    for msg in messages {
        if msg.role != MessageRole::User {
            continue;
        }
        let Some(text) = msg.content.as_str() else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if out.last().is_some_and(|last| last == trimmed) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    while out.len() > 200 {
        out.remove(0);
    }
    out
}

pub(super) enum ChatNav {
    Stay,
    ToSessions,
    /// Exit the application entirely.
    Exit,
}
