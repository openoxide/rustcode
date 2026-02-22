use super::super::{
    push_toast, AgentOptions, AppState, Arc, CancellationToken, ChatFocus, ChatState, Command,
    CommandContext, Duration, EventPublisher, InteractiveMsg, InteractiveSubmitMode,
    RunningCommand, SessionMeta, SystemTime, ToastVariant, TuiPublisher,
};

/// Submit a prompt to the agent/run executor and wire up the running state.
///
/// Spawns a Tokio task on the runtime, binds the cancellation token to `chat.running`,
/// and sends a `RunEnded` message when the task completes.
pub(crate) fn submit_prompt(state: &mut AppState, chat: &mut ChatState, prompt: String) {
    let Some(config) = state.config.clone() else {
        let msg = state
            .status
            .clone()
            .unwrap_or_else(|| "config not loaded — check your rustcode config file".to_string());
        state.status = Some(msg.clone());
        push_toast(state, ToastVariant::Error, msg, Duration::from_secs(6));
        return;
    };
    let Some(executor) = state.executor.clone() else {
        // Preserve the original init error (e.g. missing API key) if already set
        let msg = state
            .status
            .clone()
            .unwrap_or_else(|| "LLM executor not available — check API key in config".to_string());
        state.status = Some(msg.clone());
        push_toast(state, ToastVariant::Error, msg, Duration::from_secs(6));
        return;
    };

    let is_draft = crate::is_draft_session_id(&chat.session.id);
    if is_draft {
        let cwd = std::path::PathBuf::from(&chat.session.cwd);
        let workspace_root = std::path::PathBuf::from(&chat.session.workspace_root);
        let model = chat.session.model.clone();
        let title = chat.session.title.clone();
        match state.backend.create_session(crate::CreateSessionOptions {
            title,
            parent_id: None,
            cwd,
            workspace_root,
            model,
        }) {
            Ok(session) => {
                chat.session = session.clone();
                state.sessions = state.backend.list_sessions().unwrap_or_else(|_| Vec::new());
                super::super::sort_sessions(&mut state.sessions);
                state.sessions_view =
                    super::super::compute_sessions_view(&state.sessions, &state.sessions_filter);
                if let Some(idx) = state.sessions.iter().position(|s| s.id == session.id) {
                    state.selected = idx;
                }
            }
            Err(err) => {
                let msg = format!("failed to create session: {err}");
                state.status = Some(msg.clone());
                push_toast(state, ToastVariant::Error, msg, Duration::from_secs(4));
                return;
            }
        }
    }

    let history = if state.submit_mode == InteractiveSubmitMode::Agent {
        match state.backend.load_messages(&chat.session.id) {
            Ok(history) => Some(history),
            Err(err) => {
                state.status = Some(format!("failed to load history: {err}"));
                push_toast(
                    state,
                    ToastVariant::Error,
                    format!("failed to load history: {err}"),
                    Duration::from_secs(4),
                );
                return;
            }
        }
    } else {
        None
    };

    let cancellation = CancellationToken::new();
    chat.run_started_at = Some(std::time::Instant::now());
    chat.last_run_elapsed = None;
    chat.pending_prompt = Some(prompt.clone());
    chat.composer_cleared_by_ctrl_c = false;
    chat.scroll = 0; // auto-scroll to bottom on new prompt
    state.status = None;
    chat.live_assistant.clear();
    chat.activity.clear();
    chat.activity_selected = 0;

    if rustcode_state::push_prompt_history_entry(&mut state.global_prompt_history, &prompt) {
        if let Err(err) = state.prompt_history_store.append(&prompt) {
            push_toast(
                state,
                ToastVariant::Warning,
                format!("failed to save prompt history: {err}"),
                Duration::from_secs(3),
            );
        }
    }
    state.history_cursor = None;
    state.history_draft.clear();
    chat.focus = ChatFocus::Composer;

    state.request_seq = state.request_seq.saturating_add(1);
    let seq = state.request_seq;
    let session_id = chat.session.id.clone();
    let request_id = format!("tui-{seq}");

    let tx = state.tx.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(TuiPublisher::new(tx.clone()));
    let submit_mode = state.submit_mode;
    let mode_hint = state.approval_mode.mode_hint();
    let cancellation_for_task = cancellation.clone();
    let run_task = tokio::spawn(async move {
        let context = CommandContext::with_cancellation(
            config,
            SessionMeta {
                session_id,
                request_id,
                started_at: SystemTime::now(),
            },
            cancellation_for_task,
        );

        let command = match submit_mode {
            InteractiveSubmitMode::Agent => {
                let options = AgentOptions {
                    allow_write: true,
                    allow_edit: true,
                    allow_exec: true,
                    mode_hint: Some(mode_hint),
                    ..AgentOptions::default()
                };
                Command::Agent {
                    prompt,
                    options,
                    history: history.unwrap_or_default(),
                }
            }
            InteractiveSubmitMode::Run => Command::Run { prompt },
        };

        let result = executor.execute(command, context, publisher).await;
        let (ok, message) = match result {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        };
        let _ = tx.send(InteractiveMsg::RunEnded { ok, message });
    });
    chat.running = Some(RunningCommand {
        cancellation,
        abort_handle: run_task.abort_handle(),
    });
}

/// Submit a compact command to the engine to summarize context.
///
/// Like `submit_prompt`, but sends `Command::Compact` instead of an agent prompt.
pub(crate) fn submit_compact(
    state: &mut AppState,
    chat: &mut ChatState,
    focus: Option<String>,
) {
    let Some(config) = state.config.clone() else {
        push_toast(
            state,
            ToastVariant::Error,
            "config not loaded",
            Duration::from_secs(4),
        );
        return;
    };
    let Some(executor) = state.executor.clone() else {
        push_toast(
            state,
            ToastVariant::Error,
            "LLM executor not available",
            Duration::from_secs(4),
        );
        return;
    };

    let history = match state.backend.load_messages(&chat.session.id) {
        Ok(h) => h,
        Err(err) => {
            push_toast(
                state,
                ToastVariant::Error,
                format!("failed to load history: {err}"),
                Duration::from_secs(4),
            );
            return;
        }
    };

    if history.is_empty() {
        push_toast(
            state,
            ToastVariant::Info,
            "nothing to compact — conversation is empty",
            Duration::from_secs(3),
        );
        return;
    }

    let cancellation = CancellationToken::new();
    chat.run_started_at = Some(std::time::Instant::now());
    chat.last_run_elapsed = None;
    chat.scroll = 0;
    state.status = None;
    chat.live_assistant.clear();
    chat.activity.clear();
    chat.activity_selected = 0;

    state.request_seq = state.request_seq.saturating_add(1);
    let seq = state.request_seq;
    let session_id = chat.session.id.clone();
    let request_id = format!("tui-compact-{seq}");

    let tx = state.tx.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(TuiPublisher::new(tx.clone()));
    let cancellation_for_task = cancellation.clone();
    let run_task = tokio::spawn(async move {
        let context = CommandContext::with_cancellation(
            config,
            SessionMeta {
                session_id,
                request_id,
                started_at: SystemTime::now(),
            },
            cancellation_for_task,
        );

        let command = Command::Compact { history, focus };
        let result = executor.execute(command, context, publisher).await;
        let (ok, message) = match result {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        };
        let _ = tx.send(InteractiveMsg::RunEnded { ok, message });
    });
    chat.running = Some(RunningCommand {
        cancellation,
        abort_handle: run_task.abort_handle(),
    });
}
