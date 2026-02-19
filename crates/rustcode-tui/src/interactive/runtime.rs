use super::{
    build_prompt_history, composer_insert_str, compute_sessions_view, disable_raw_mode,
    drain_toasts, enable_raw_mode, event, execute, handle_key, io, push_toast, render,
    sort_sessions, ActivityItem, AgentOptions, AppState, Arc, CEvent, CancellationToken, ChatFocus,
    ChatState, Command, CommandContext, CrosstermBackend, Duration, EnterAlternateScreen,
    EventPayload, EventPublisher, ExecutableCommand, InteractiveMsg, InteractiveServices,
    InteractiveStart, InteractiveSubmitMode, KeyEventKind, LeaveAlternateScreen, PendingApproval,
    RunningCommand, Screen, SessionMeta, SystemTime, Terminal, ToastVariant, TuiError,
    TuiPublisher,
};

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
        submit_mode,
    } = services;

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    execute!(stdout, EnterAlternateScreen).map_err(|err| TuiError::Io(err.to_string()))?;
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
                CEvent::Paste(text) => handle_paste(&mut state, &text),
                _ => {}
            }
        }
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
                            state.status = Some(message.clone());
                            push_toast(
                                state,
                                ToastVariant::Error,
                                message.clone(),
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
                            Ok(messages) => chat.messages = messages,
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
                    chat.live_assistant.clear();
                    if ok {
                        state.status = None;
                    } else {
                        state.status = message;
                        if let Some(msg) = state.status.clone() {
                            push_toast(state, ToastVariant::Error, msg, Duration::from_secs(6));
                        }
                    }
                    match state.backend.load_messages(&chat.session.id) {
                        Ok(messages) => chat.messages = messages,
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
        state.status = Some("config not loaded; cannot run".to_string());
        push_toast(
            state,
            ToastVariant::Error,
            "config not loaded; cannot run",
            Duration::from_secs(4),
        );
        return;
    };
    let Some(executor) = state.executor.clone() else {
        state.status = Some("executor not available; cannot run".to_string());
        push_toast(
            state,
            ToastVariant::Error,
            "executor not available; cannot run",
            Duration::from_secs(4),
        );
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

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = stdout.execute(LeaveAlternateScreen);
    }
}
