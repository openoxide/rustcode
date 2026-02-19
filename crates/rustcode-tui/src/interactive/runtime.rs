use super::{
    build_prompt_history, centered_rect, composer_insert_str, compute_sessions_view,
    disable_raw_mode, drain_toasts, enable_raw_mode, event, execute, handle_key, io, push_toast,
    render, sort_sessions, ActivityItem, AgentOptions, AppState, Arc, CEvent, CancellationToken,
    ChatFocus, ChatState, Command, CommandContext, Constraint, CrosstermBackend, Direction,
    DisableMouseCapture, Duration, EnableMouseCapture, EnterAlternateScreen, EventPayload,
    EventPublisher, ExecutableCommand, InteractiveMsg, InteractiveServices, InteractiveStart,
    InteractiveSubmitMode, KeyEventKind, Layout, LeaveAlternateScreen, Modal, MouseButton,
    MouseEvent, MouseEventKind, PendingApproval, ProviderManagerStep, Rect, RunningCommand, Screen,
    SessionMeta, SystemTime, Terminal, ToastVariant, TuiError, TuiPublisher,
};
use rustcode_core::event::EventScope;

pub(super) fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    let InteractiveServices {
        backend: session_backend,
        defaults,
        initial_status,
        start,
        runtime,
        handles,
        config,
        executor,
        llm_cell,
        submit_mode,
    } = services;

    // Install a terminal-restoring panic hook so the panic message is legible.
    // Without this, the message prints while the terminal is still in raw mode.
    // TerminalCleanup's Drop still runs during unwinding, so double-restore is harmless.
    let prev_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.execute(LeaveAlternateScreen);
        prev_panic_hook(info);
    }));

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|err| TuiError::Io(err.to_string()))?;
    let _cleanup = TerminalCleanup;

    let term_backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(term_backend).map_err(|err| TuiError::Io(err.to_string()))?;
    terminal
        .clear()
        .map_err(|err| TuiError::Io(err.to_string()))?;

    let mut sessions = session_backend.list_sessions().map_err(TuiError::State)?;
    sort_sessions(&mut sessions);
    let sessions_view = compute_sessions_view(&sessions, "");
    let mut selected = 0usize;
    let mut screen = Screen::Sessions;
    let mut auto_submit = None;

    match start {
        InteractiveStart::Sessions => {}
        InteractiveStart::Chat {
            session,
            prompt,
            auto_submit: should_submit,
        } => {
            if let Some(idx) = sessions
                .iter()
                .position(|candidate| candidate.id == session.id)
            {
                selected = idx;
            }
            let messages = session_backend
                .load_messages(&session.id)
                .map_err(TuiError::State)?;
            let prompt_history = build_prompt_history(&messages);
            let initial_composer = if should_submit {
                String::new()
            } else {
                prompt.clone().unwrap_or_default()
            };
            screen = Screen::Chat(ChatState {
                session,
                messages,
                scroll: 0,
                live_assistant: String::new(),
                composer_cursor: initial_composer.len(),
                composer: initial_composer,
                prompt_history,
                history_cursor: None,
                history_draft: String::new(),
                focus: ChatFocus::Composer,
                activity: Vec::new(),
                activity_selected: 0,
                details_open: false,
                tool_details: false,
                find: None,
                running: None,
                pending_prompt: None,
                composer_cleared_by_ctrl_c: false,
                last_typing_time: None,
            });
            if should_submit {
                auto_submit = prompt;
            }
        }
    }

    let mut state = AppState {
        sessions,
        sessions_view,
        selected,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen,
        status: initial_status,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults,
        pending_approval: None,
        submit_mode,
        backend: session_backend,
        config,
        executor,
        runtime,
        tx: handles.tx,
        rx: handles.rx,
        request_seq: 0,
        last_area: terminal
            .size()
            .map_err(|err| TuiError::Io(err.to_string()))?,
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell,
    };

    if let Some(prompt) = auto_submit {
        if !prompt.trim().is_empty() {
            start_auto_submit(&mut state, prompt);
        }
    }

    loop {
        if let Ok(size) = terminal.size() {
            state.last_area = size;
        }
        drain_toasts(&mut state);
        drain_messages(&mut state);
        poll_provider_oauth(&mut state);

        terminal
            .draw(|frame| render(frame, &state))
            .map_err(|err| TuiError::Io(err.to_string()))?;

        if event::poll(Duration::from_millis(50)).map_err(|err| TuiError::Io(err.to_string()))? {
            let evt = event::read().map_err(|err| TuiError::Io(err.to_string()))?;
            match evt {
                CEvent::Key(key) => {
                    if key.kind == KeyEventKind::Press && handle_key(&mut state, key) {
                        return Ok(());
                    }
                }
                CEvent::Mouse(mouse) => handle_mouse(&mut state, mouse),
                CEvent::Paste(text) => handle_paste(&mut state, &text),
                _ => {}
            }
        }
    }
}

pub(super) fn handle_mouse(state: &mut AppState, mouse: MouseEvent) {
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }

    // Allow clicks inside the Feedback modal to select rating buttons
    if matches!(&state.modal, Some(Modal::Feedback { .. })) {
        handle_feedback_mouse(state, mouse);
        return;
    }

    if state.pending_approval.is_some() || state.modal.is_some() || state.help_open {
        return;
    }

    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    if chat.details_open {
        state.screen = Screen::Chat(chat);
        return;
    }

    let frame_area = Rect::new(0, 0, state.last_area.width, state.last_area.height);
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(frame_area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(3),
        ])
        .split(root[0]);

    let col = mouse.column;
    let row = mouse.row;
    if contains(left[0], col, row) {
        chat.focus = ChatFocus::Transcript;
    } else if contains(left[1], col, row) {
        chat.focus = ChatFocus::Composer;
    } else if contains(root[1], col, row) {
        chat.focus = ChatFocus::Activity;
    }

    state.screen = Screen::Chat(chat);
}

fn contains(area: Rect, col: u16, row: u16) -> bool {
    let max_x = area.x.saturating_add(area.width);
    let max_y = area.y.saturating_add(area.height);
    col >= area.x && col < max_x && row >= area.y && row < max_y
}

/// Handle a left-click inside the Feedback modal.
///
/// The rating block layout (after the outer modal border and rating block border):
/// - Line 0: "How useful was this session?"
/// - Line 1: (empty)
/// - Line 2: " [+] thumbs up " (16 chars)  "   " (3)  " [-] thumbs down " (18 chars)
///
/// Button row y = modal.y + 1 (outer border) + 1 (rating block top border) + 2 (line index)
///             = modal.y + 4
/// Up button   x = modal.x + 1 (outer) + 1 (rating border) = modal.x + 2, width 16
/// Down button x = modal.x + 2 + 16 + 3 = modal.x + 21, width 18
fn handle_feedback_mouse(state: &mut AppState, mouse: MouseEvent) {
    let screen_rect = Rect::new(0, 0, state.last_area.width, state.last_area.height);
    let modal_rect = centered_rect(68, 52, screen_rect);

    let button_row = modal_rect.y.saturating_add(4);
    let up_x_start = modal_rect.x.saturating_add(2);
    let up_x_end = up_x_start.saturating_add(16);
    let down_x_start = up_x_end.saturating_add(3);
    let down_x_end = down_x_start.saturating_add(18);

    let col = mouse.column;
    let row = mouse.row;

    if row != button_row {
        return;
    }

    let clicked_up = col >= up_x_start && col < up_x_end;
    let clicked_down = col >= down_x_start && col < down_x_end;

    if !clicked_up && !clicked_down {
        return;
    }

    if let Some(Modal::Feedback { ref mut rating, .. }) = state.modal {
        *rating = Some(clicked_up);
    }
}

pub(super) fn handle_paste(state: &mut AppState, text: &str) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    if chat.focus == ChatFocus::Composer {
        composer_insert_str(&mut chat, &normalized);
    }
    state.screen = Screen::Chat(chat);
}

pub(super) fn start_auto_submit(state: &mut AppState, prompt: String) {
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    submit_prompt(state, &mut chat, prompt);
    state.screen = Screen::Chat(chat);
}

pub(super) fn drain_messages(state: &mut AppState) {
    loop {
        let msg = match state.rx.try_recv() {
            Ok(msg) => msg,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
        };

        let mut screen = std::mem::replace(&mut state.screen, Screen::Sessions);
        match msg {
            InteractiveMsg::EngineEvent(event) => {
                if let Screen::Chat(chat) = &mut screen {
                    let mut refresh = false;
                    let item = match &event.payload {
                        EventPayload::CommandAccepted { name } => {
                            Some(ActivityItem::CommandAccepted { name: name.clone() })
                        }
                        EventPayload::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(ActivityItem::ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            arguments: arguments.clone(),
                        }),
                        EventPayload::ToolResult {
                            id,
                            name,
                            ok,
                            output,
                        } => Some(ActivityItem::ToolResult {
                            id: id.clone(),
                            name: name.clone(),
                            ok: *ok,
                            output: output.clone(),
                        }),
                        EventPayload::OutputChunk { text } => {
                            if event.scope == EventScope::Command {
                                chat.live_assistant.push_str(text);
                                if chat.live_assistant.len() > 64 * 1024 {
                                    let keep = 48 * 1024;
                                    let mut start = chat.live_assistant.len().saturating_sub(keep);
                                    while start < chat.live_assistant.len()
                                        && !chat.live_assistant.is_char_boundary(start)
                                    {
                                        start += 1;
                                    }
                                    chat.live_assistant = chat.live_assistant[start..].to_string();
                                }
                            }
                            Some(ActivityItem::OutputChunk { text: text.clone() })
                        }
                        EventPayload::Warning { message } => {
                            push_toast(
                                state,
                                ToastVariant::Warning,
                                message.clone(),
                                Duration::from_secs(4),
                            );
                            Some(ActivityItem::Warning {
                                message: message.clone(),
                            })
                        }
                        EventPayload::Failure { message } => {
                            // Show a clean, actionable message in the footer and
                            // toast; preserve the full technical detail in the
                            // Activity panel (Ctrl+E to expand).
                            let clean = clean_failure_message(&message);
                            state.status = Some(clean.clone());
                            push_toast(
                                state,
                                ToastVariant::Error,
                                clean,
                                Duration::from_secs(6),
                            );
                            refresh = true;
                            chat.live_assistant.clear();
                            Some(ActivityItem::Failure {
                                message: message.clone(),
                            })
                        }
                        EventPayload::Completed => {
                            chat.running = None;
                            refresh = true;
                            chat.live_assistant.clear();
                            push_toast(
                                state,
                                ToastVariant::Success,
                                "completed",
                                Duration::from_secs(2),
                            );
                            Some(ActivityItem::Completed)
                        }
                        EventPayload::ServeRequest { .. } => None,
                    };

                    if let Some(item) = item {
                        chat.activity.push(item);
                    }
                    while chat.activity.len() > 200 {
                        chat.activity.remove(0);
                        if chat.activity_selected > 0 {
                            chat.activity_selected -= 1;
                        }
                    }
                    if chat.activity_selected >= chat.activity.len() {
                        chat.activity_selected = chat.activity.len().saturating_sub(1);
                    }

                    if refresh {
                        match state.backend.load_messages(&chat.session.id) {
                            Ok(messages) => {
                                chat.messages = messages;
                                chat.scroll = 0;
                            }
                            Err(err) => {
                                state.status = Some(format!("failed to load transcript: {err}"));
                            }
                        }
                    }
                }
            }
            InteractiveMsg::ApprovalRequest { request, reply } => {
                state.pending_approval = Some(PendingApproval { request, reply });
            }
            InteractiveMsg::RunEnded { ok, message } => {
                if let Screen::Chat(chat) = &mut screen {
                    chat.running = None;
                    chat.pending_prompt = None;
                    chat.live_assistant.clear();
                    if ok {
                        state.status = None;
                    } else {
                        let clean = message.as_deref().map(clean_failure_message);
                        state.status = clean.clone();
                        if let Some(msg) = clean {
                            push_toast(state, ToastVariant::Error, msg, Duration::from_secs(6));
                        }
                    }
                    match state.backend.load_messages(&chat.session.id) {
                        Ok(messages) => {
                            chat.messages = messages;
                            chat.scroll = 0;
                        }
                        Err(err) => {
                            state.status = Some(format!("failed to load transcript: {err}"));
                            push_toast(
                                state,
                                ToastVariant::Error,
                                format!("failed to load transcript: {err}"),
                                Duration::from_secs(4),
                            );
                        }
                    }
                }
            }
        }

        state.screen = screen;
    }
}

pub(super) fn submit_prompt(state: &mut AppState, chat: &mut ChatState, prompt: String) {
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
    chat.running = Some(RunningCommand {
        cancellation: cancellation.clone(),
    });
    chat.pending_prompt = Some(prompt.clone());
    state.status = None;
    chat.live_assistant.clear();

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
    chat.focus = ChatFocus::Transcript;

    state.request_seq = state.request_seq.saturating_add(1);
    let seq = state.request_seq;
    let session_id = chat.session.id.clone();
    let request_id = format!("tui-{seq}");

    let tx = state.tx.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(TuiPublisher::new(tx.clone()));
    let runtime = state.runtime.clone();
    let submit_mode = state.submit_mode;
    runtime.spawn(async move {
        let context = CommandContext::with_cancellation(
            config,
            SessionMeta {
                session_id,
                request_id,
                started_at: SystemTime::now(),
            },
            cancellation,
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

        let result = executor.execute(command, context, publisher).await;
        let (ok, message) = match result {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        };
        let _ = tx.send(InteractiveMsg::RunEnded { ok, message });
    });
}

/// Poll the OAuth background-task channels and update the provider manager modal.
///
/// Called every loop iteration (50 ms). Non-blocking — uses `try_recv`.
pub(super) fn poll_provider_oauth(state: &mut AppState) {
    // Poll the "device code started" channel.
    if let Some(rx) = &state.provider_oauth_start_rx {
        match rx.try_recv() {
            Ok(Ok(started)) => {
                if let Some(Modal::ProviderManager { step }) = &mut state.modal {
                    if let ProviderManagerStep::OAuthStarting { display_name, .. } = step {
                        let display_name = display_name.clone();
                        *step = ProviderManagerStep::OAuthPending {
                            provider_id: started.provider_id,
                            display_name,
                            verification_uri: started.verification_uri,
                            user_code: started.user_code,
                        };
                    }
                }
                state.provider_oauth_start_rx = None;
            }
            Ok(Err(err)) => {
                push_toast(
                    state,
                    ToastVariant::Error,
                    format!("OAuth failed to start: {err}"),
                    Duration::from_secs(6),
                );
                state.modal = None;
                state.provider_oauth_start_rx = None;
                state.provider_oauth_done_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.provider_oauth_start_rx = None;
            }
        }
    }

    // Poll the "credential ready" channel.
    if let Some(rx) = &state.provider_oauth_done_rx {
        match rx.try_recv() {
            Ok(Ok(done)) => {
                let store = rustcode_auth::AuthStore::open_default();
                match store.set_oauth(
                    &done.provider_id,
                    &done.access_token,
                    done.refresh_token.as_deref(),
                    done.expires_at_unix,
                    done.account_id.as_deref(),
                ) {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Success,
                        format!("{} connected!", done.provider_id),
                        Duration::from_secs(4),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to save credential: {err}"),
                        Duration::from_secs(6),
                    ),
                }
                state.modal = None;
                state.provider_oauth_done_rx = None;
            }
            Ok(Err(err)) => {
                push_toast(
                    state,
                    ToastVariant::Error,
                    format!("OAuth authorization failed: {err}"),
                    Duration::from_secs(6),
                );
                state.modal = None;
                state.provider_oauth_done_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.provider_oauth_done_rx = None;
            }
        }
    }
}

/// Convert a raw executor/LLM error string into a short, actionable message
/// suitable for the status footer and toast notification.
///
/// The full technical detail is preserved in `ActivityItem::Failure` for
/// debugging via Ctrl+E.  The pattern matching targets the `{kind:?}` Debug
/// representations embedded by `LlmError::Classified`'s Display impl.
fn clean_failure_message(msg: &str) -> String {
    let lower = msg.to_lowercase();

    // Auth failures — API key missing or rejected
    if lower.contains("authfailed")
        || lower.contains("authentication failed")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || (lower.contains("401") && lower.contains("provider"))
    {
        return "Authentication failed — connect your provider in /providers (Ctrl+A)".to_string();
    }

    // Rate limit / quota exhausted
    if lower.contains("ratelimit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
        || lower.contains("429")
    {
        return "Rate limit or usage quota reached — please wait before retrying".to_string();
    }

    // Context window overflow
    if lower.contains("contextoverflow")
        || lower.contains("context limit")
        || lower.contains("context_length_exceeded")
        || lower.contains("context window")
        || lower.contains("maximum context")
    {
        return "Context limit exceeded — use /compact or send a shorter message".to_string();
    }

    // Provider service temporarily unavailable
    if lower.contains("serviceunavailable")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("overloaded")
        || lower.contains("internal server error")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
    {
        return "Provider temporarily unavailable — please try again".to_string();
    }

    // Invalid request
    if lower.contains("invalidrequest") {
        return "Request rejected by provider — see activity for details".to_string();
    }

    // Config / no executor — no provider connected
    if lower.contains("configuration error")
        || lower.contains("not set")
        || lower.contains("api key env")
        || lower.contains("executor not available")
        || lower.contains("llm init failed")
    {
        return "No provider connected — use /providers (Ctrl+A) to connect one".to_string();
    }

    // Network / transport errors
    if lower.contains("network error")
        || lower.contains("connection")
        || lower.contains("timed out")
        || lower.contains("transport")
    {
        return "Network error — check your connection and try again".to_string();
    }

    // Generic: strip "provider error (X): " prefix to expose the clean body
    if let Some(idx) = msg.find("): ") {
        let rest = msg[idx + 3..].trim();
        if !rest.is_empty() && rest.len() < 120 {
            return format!("Request failed: {rest}");
        }
    }

    // Last resort: return as-is but capped at 120 chars
    let capped: String = msg.chars().take(120).collect();
    if capped.len() < msg.len() {
        format!("{capped}…")
    } else {
        capped
    }
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.execute(LeaveAlternateScreen);
    }
}
