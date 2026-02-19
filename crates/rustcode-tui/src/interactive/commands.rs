use super::{
    build_prompt_history, build_transcript_lines, composer_clear, compute_find_matches,
    compute_sessions_view, push_toast, sort_sessions, AppState, ChatFocus, ChatNav, ChatState,
    CommandId, CreateSessionOptions, Duration, Modal, Screen, ToastVariant,
};

pub(super) fn execute_command(state: &mut AppState, id: CommandId) {
    match id {
        CommandId::Help => state.help_open = true,
        CommandId::Sessions => state.screen = Screen::Sessions,
        CommandId::NewSession => {
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to resolve cwd: {err}"),
                        Duration::from_secs(4),
                    );
                    return;
                }
            };
            match state.backend.create_session(CreateSessionOptions {
                title: None,
                parent_id: None,
                cwd,
                workspace_root: state.defaults.workspace_root.clone(),
                model: state.defaults.model.clone(),
            }) {
                Ok(session) => {
                    state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                        push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                        Vec::new()
                    });
                    sort_sessions(&mut state.sessions);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = state
                        .sessions
                        .iter()
                        .position(|s| s.id == session.id)
                        .unwrap_or(0);

                    let messages = state
                        .backend
                        .load_messages(&session.id)
                        .unwrap_or_else(|err| {
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    state.screen = Screen::Chat(ChatState {
                        session,
                        messages,
                        scroll: 0,
                        live_assistant: String::new(),
                        composer: String::new(),
                        composer_cursor: 0,
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
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "created session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to create session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
        }
        CommandId::ForkSession => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
            match state.backend.fork_session(&chat.session.id, None) {
                Ok(forked) => {
                    let messages = state
                        .backend
                        .load_messages(&forked.id)
                        .unwrap_or_else(|err| {
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    chat.session = forked;
                    chat.messages = messages;
                    chat.scroll = 0;
                    chat.live_assistant.clear();
                    composer_clear(&mut chat);
                    chat.prompt_history = build_prompt_history(&chat.messages);
                    chat.history_cursor = None;
                    chat.history_draft.clear();
                    chat.focus = ChatFocus::Composer;
                    chat.activity.clear();
                    chat.activity_selected = 0;
                    chat.details_open = false;
                    chat.tool_details = false;
                    chat.find = None;
                    chat.running = None;
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "forked session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => push_toast(state, ToastVariant::Error, err, Duration::from_secs(4)),
            }
            state.screen = Screen::Chat(chat);
        }
        CommandId::RenameSession => match &state.screen {
            Screen::Sessions => {
                let Some(session) = state
                    .sessions_view
                    .get(state.selected)
                    .and_then(|idx| state.sessions.get(*idx))
                    .cloned()
                else {
                    return;
                };
                let input = session.title.clone().unwrap_or_default();
                state.modal = Some(Modal::Rename {
                    session_id: session.id,
                    cursor: input.chars().count(),
                    input,
                });
            }
            Screen::Chat(chat) => {
                let input = chat.session.title.clone().unwrap_or_default();
                state.modal = Some(Modal::Rename {
                    session_id: chat.session.id.clone(),
                    cursor: input.chars().count(),
                    input,
                });
            }
        },
        CommandId::DeleteSession => {
            let (session_id, title) = match &state.screen {
                Screen::Sessions => {
                    let Some(session) = state
                        .sessions_view
                        .get(state.selected)
                        .and_then(|idx| state.sessions.get(*idx))
                        .cloned()
                    else {
                        return;
                    };
                    (
                        session.id.clone(),
                        session.title.clone().unwrap_or_else(|| session.id.clone()),
                    )
                }
                Screen::Chat(chat) => (
                    chat.session.id.clone(),
                    chat.session
                        .title
                        .clone()
                        .unwrap_or_else(|| chat.session.id.clone()),
                ),
            };
            state.modal = Some(Modal::DeleteConfirm { session_id, title });
        }
        CommandId::Refresh => match &state.screen {
            Screen::Sessions => {
                state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                    push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                    Vec::new()
                });
                sort_sessions(&mut state.sessions);
                state.sessions_view =
                    compute_sessions_view(&state.sessions, &state.sessions_filter);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "refreshed sessions",
                    Duration::from_secs(2),
                );
            }
            Screen::Chat(_) => {
                let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
                else {
                    return;
                };
                refresh_chat_messages(state, &mut chat);
                state.screen = Screen::Chat(chat);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "refreshed transcript",
                    Duration::from_secs(2),
                );
            }
        },
        CommandId::ToggleTools => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
            chat.tool_details = !chat.tool_details;
            push_toast(
                state,
                ToastVariant::Info,
                if chat.tool_details {
                    "tools: details"
                } else {
                    "tools: summary"
                },
                Duration::from_secs(2),
            );
            state.screen = Screen::Chat(chat);
        }
        CommandId::Search => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
            chat.focus = ChatFocus::Transcript;
            let query = chat
                .find
                .as_ref()
                .map(|find| find.query.clone())
                .unwrap_or_default();
            let transcript = build_transcript_lines(&chat);
            let matches = compute_find_matches(&transcript, &query);
            let current = chat.find.as_ref().map_or(0, |f| f.current);
            state.screen = Screen::Chat(chat);
            state.modal = Some(Modal::Search {
                query,
                current,
                matches,
            });
        }
        CommandId::FocusComposer => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Composer;
            }
        }
        CommandId::FocusTranscript => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Transcript;
            }
        }
        CommandId::FocusActivity => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Activity;
            }
        }
        CommandId::CancelRun => {
            if let Screen::Chat(chat) = &mut state.screen {
                if let Some(running) = &chat.running {
                    running.cancellation.cancel();
                    push_toast(
                        state,
                        ToastVariant::Warning,
                        "cancel requested",
                        Duration::from_secs(2),
                    );
                }
            }
        }
        CommandId::Quit => push_toast(
            state,
            ToastVariant::Info,
            "press q to quit",
            Duration::from_secs(2),
        ),
    }
}

pub(super) fn handle_slash_command(
    state: &mut AppState,
    chat: &mut ChatState,
    input: &str,
) -> ChatNav {
    let raw = input.trim();
    let cmd = raw.trim_start_matches('/').trim();
    match cmd {
        "help" => {
            state.help_open = true;
            ChatNav::Stay
        }
        "sessions" | "home" => ChatNav::ToSessions,
        "clear" => {
            composer_clear(chat);
            ChatNav::Stay
        }
        "reload" | "refresh" => {
            refresh_chat_messages(state, chat);
            push_toast(
                state,
                ToastVariant::Info,
                "refreshed transcript",
                Duration::from_secs(2),
            );
            ChatNav::Stay
        }
        "tools" => {
            chat.tool_details = !chat.tool_details;
            push_toast(
                state,
                ToastVariant::Info,
                if chat.tool_details {
                    "tools: details"
                } else {
                    "tools: summary"
                },
                Duration::from_secs(2),
            );
            ChatNav::Stay
        }
        "new" => {
            if chat.running.is_some() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "cannot create session while running",
                    Duration::from_secs(3),
                );
                return ChatNav::Stay;
            }
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to resolve cwd: {err}"),
                        Duration::from_secs(4),
                    );
                    return ChatNav::Stay;
                }
            };
            match state.backend.create_session(CreateSessionOptions {
                title: None,
                parent_id: None,
                cwd,
                workspace_root: state.defaults.workspace_root.clone(),
                model: state.defaults.model.clone(),
            }) {
                Ok(session) => {
                    let messages = state
                        .backend
                        .load_messages(&session.id)
                        .unwrap_or_else(|err| {
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    chat.session = session;
                    chat.messages = messages;
                    chat.scroll = 0;
                    chat.live_assistant.clear();
                    composer_clear(chat);
                    chat.prompt_history = build_prompt_history(&chat.messages);
                    chat.history_cursor = None;
                    chat.history_draft.clear();
                    chat.focus = ChatFocus::Composer;
                    chat.activity.clear();
                    chat.activity_selected = 0;
                    chat.details_open = false;
                    chat.tool_details = false;
                    chat.running = None;
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "created session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to create session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
            ChatNav::Stay
        }
        "fork" => {
            if chat.running.is_some() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "cannot fork session while running",
                    Duration::from_secs(3),
                );
                return ChatNav::Stay;
            }
            match state.backend.fork_session(&chat.session.id, None) {
                Ok(forked) => {
                    let messages = state
                        .backend
                        .load_messages(&forked.id)
                        .unwrap_or_else(|err| {
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    chat.session = forked;
                    chat.messages = messages;
                    chat.scroll = 0;
                    chat.live_assistant.clear();
                    composer_clear(chat);
                    chat.prompt_history = build_prompt_history(&chat.messages);
                    chat.history_cursor = None;
                    chat.history_draft.clear();
                    chat.focus = ChatFocus::Composer;
                    chat.activity.clear();
                    chat.activity_selected = 0;
                    chat.details_open = false;
                    chat.tool_details = false;
                    chat.running = None;
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "forked session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to fork session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
            ChatNav::Stay
        }
        _ => {
            push_toast(
                state,
                ToastVariant::Warning,
                format!("unknown command: {raw}"),
                Duration::from_secs(3),
            );
            ChatNav::Stay
        }
    }
}

pub(super) fn refresh_chat_messages(state: &mut AppState, chat: &mut ChatState) {
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
