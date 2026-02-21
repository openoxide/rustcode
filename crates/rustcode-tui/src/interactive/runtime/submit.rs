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

    let trimmed = prompt.trim();
    if !trimmed.is_empty()
        && chat
            .prompt_history
            .last()
            .is_none_or(|last| last.as_str() != trimmed)
    {
        chat.prompt_history.push(trimmed.to_string());
        while chat.prompt_history.len() > 200 {
            chat.prompt_history.remove(0);
        }
    }
    chat.history_cursor = None;
    chat.history_draft.clear();
    chat.focus = ChatFocus::Composer;

    state.request_seq = state.request_seq.saturating_add(1);
    let seq = state.request_seq;
    let session_id = chat.session.id.clone();
    let request_id = format!("tui-{seq}");

    let tx = state.tx.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(TuiPublisher::new(tx.clone()));
    let runtime = state.runtime.clone();
    let submit_mode = state.submit_mode;
    let cancellation_for_task = cancellation.clone();
    let run_task = runtime.spawn(async move {
        let cancel_timeout = cancellation_for_task.clone();
        let tx_timeout = tx.clone();

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

        tokio::select! {
            result = executor.execute(command, context, publisher) => {
                let (ok, message) = match result {
                    Ok(()) => (true, None),
                    Err(err) => (false, Some(err.to_string())),
                };
                let _ = tx.send(InteractiveMsg::RunEnded { ok, message });
            }
            () = tokio::time::sleep(std::time::Duration::from_secs(7 * 60)) => {
                cancel_timeout.cancel();
                let _ = tx_timeout.send(InteractiveMsg::RunEnded {
                    ok: false,
                    message: Some("\u{23f1} Run timed out after 7 minutes".to_string()),
                });
            }
        }
    });
    chat.running = Some(RunningCommand {
        cancellation,
        abort_handle: run_task.abort_handle(),
    });
}
