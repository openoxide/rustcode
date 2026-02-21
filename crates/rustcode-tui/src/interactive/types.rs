use std::collections::VecDeque;

use super::{
    AbortHandle, ActivityItem, ApprovalResponse, Arc, CancellationToken, InteractiveDefaults,
    InteractiveMsg, InteractiveSubmitMode, MessageRole, ResolvedConfig, SessionInfo, Size,
    StoredMessage, SystemTime, ToolApprovalRequest,
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
    pub(super) abort_handle: AbortHandle,
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
    /// Slash-command autocomplete popup shown while composer starts with `/`.
    SlashHelp {
        /// Text after `/` used for filtering.
        query: String,
        /// Currently highlighted row index within the filtered list.
        selected: usize,
    },
    /// Full-screen scrollable memory viewer.
    MemoryViewer {
        /// Full memory summary content (or placeholder text).
        content: String,
        /// Number of raw memory files on disk.
        raw_count: usize,
        /// Unix timestamp of last summary update (0 if none).
        updated_at: u64,
        /// Whether memory collection is currently enabled.
        enabled: bool,
        /// Current vertical scroll offset (line index).
        scroll: usize,
        /// Total content line count (computed once on open).
        total_lines: usize,
    },
    /// Confirmation dialog before clearing all memories.
    MemoryClearConfirm,
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
    ToggleReasoning,
    Search,
    FileSearch,
    /// Toggle activity panel visibility.
    ToggleActivity,
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
    /// Open memory viewer overlay.
    ViewMemory,
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
    Activity,
}

pub(super) struct ChatState {
    pub(super) session: SessionInfo,
    pub(super) messages: Vec<StoredMessage>,
    pub(super) scroll: usize,
    pub(super) live_assistant: String,
    pub(super) live_reasoning: String,
    /// Whether reasoning/thinking blocks are visible in transcript.
    pub(super) show_reasoning: bool,
    pub(super) composer: String,
    pub(super) composer_cursor: usize,
    /// Full original text of a large paste — shown as a summary in `composer`.
    ///
    /// Set by `handle_paste` when the pasted text exceeds the display threshold.
    /// Used by the submit handler so the AI receives the full text.
    /// Cleared on any manual edit to the composer, or after submit.
    pub(super) paste_buffer: Option<String>,
    pub(super) prompt_history: Vec<String>,
    pub(super) history_cursor: Option<usize>,
    pub(super) history_draft: String,
    pub(super) focus: ChatFocus,
    pub(super) activity: VecDeque<ActivityItem>,
    pub(super) activity_selected: usize,
    pub(super) details_open: bool,
    /// Whether the activity panel is hidden (toggled with Ctrl+W).
    pub(super) activity_hidden: bool,
    /// Whether tool-call batches are expanded in transcript.
    pub(super) tool_details: bool,
    /// Whether tool outputs are fully expanded (vs collapsed preview).
    pub(super) output_details: bool,
    pub(super) find: Option<FindState>,
    pub(super) running: Option<RunningCommand>,
    /// The prompt most recently submitted but not yet confirmed by backend reload.
    pub(super) pending_prompt: Option<String>,
    /// Tool approval requests that were approved but whose results are not yet
    /// loaded into `messages`. Rendered in the transcript until `RunEnded` fires.
    pub(super) committed_approvals: Vec<ToolApprovalRequest>,
    /// Whether composer was just cleared by Ctrl+C (for "press again to exit" flow).
    pub(super) composer_cleared_by_ctrl_c: bool,
    /// Timestamp of last typing activity in composer.
    pub(super) last_typing_time: Option<Instant>,
    // ── Token usage tracking (accumulated from UsageUpdate events) ───
    /// Cumulative input tokens across all LLM steps in this session.
    pub(super) total_input_tokens: u64,
    /// Cumulative output tokens.
    pub(super) total_output_tokens: u64,
    /// Most recent total token count (current context window usage).
    pub(super) last_total_tokens: u64,
    /// Context window limit in tokens.
    pub(super) context_limit: u64,
    /// Estimated cumulative cost in USD.
    pub(super) cost_usd: f64,
    /// Last `max_scroll` value computed by the render pass — updated each frame
    /// via `Cell` interior mutability so the scroll event handler can clamp
    /// `scroll` immediately without phantom over-scrolling.
    pub(super) last_max_scroll: std::cell::Cell<usize>,
    /// Instant when the current run started — cleared when the run ends.
    pub(super) run_started_at: Option<Instant>,
    /// Elapsed duration of the most recently completed run.
    pub(super) last_run_elapsed: Option<std::time::Duration>,
}

/// Cached result of `git diff --shortstat HEAD`.
pub(super) struct GitStat {
    pub(super) files: u32,
    pub(super) insertions: u32,
    pub(super) deletions: u32,
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
    /// Index of the currently highlighted option in the inline approval selector.
    pub(super) approval_selection: usize,

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
    pub(super) llm_cell: Option<Arc<std::sync::RwLock<Arc<dyn rustcode_llm::LlmClient>>>>,
    /// Cached git diff stats — refreshed after each successful run completion.
    pub(super) git_stat: Option<GitStat>,
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
