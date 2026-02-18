use super::{AppState, KeyEvent, KeyCode, compute_sessions_view, push_toast, ToastVariant, Duration, build_prompt_history, Screen, ChatState, ChatFocus, KeyModifiers, open_command_palette, Modal, sort_sessions, CreateSessionOptions};

pub(super) fn handle_sessions_key(state: &mut AppState, key: KeyEvent) -> bool {
    if state.sessions_filter_active {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc => {
                if state.sessions_filter.is_empty() {
                    state.sessions_filter_active = false;
                } else {
                    state.sessions_filter.clear();
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;
                }
            }
            KeyCode::Backspace => {
                state.sessions_filter.pop();
                state.sessions_view =
                    compute_sessions_view(&state.sessions, &state.sessions_filter);
                state.selected = state
                    .selected
                    .min(state.sessions_view.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(session) = state
                    .sessions_view
                    .get(state.selected)
                    .and_then(|idx| state.sessions.get(*idx))
                    .cloned()
                {
                    let messages = state
                        .backend
                        .load_messages(&session.id)
                        .unwrap_or_else(|err| {
                            state.status = Some(err.clone());
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
                }
                state.sessions_filter_active = false;
            }
            KeyCode::Down => {
                if !state.sessions_view.is_empty() {
                    state.selected =
                        (state.selected + 1).min(state.sessions_view.len().saturating_sub(1));
                }
            }
            KeyCode::Up => state.selected = state.selected.saturating_sub(1),
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    state.sessions_filter.push(ch);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;
                }
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Char('?') => state.help_open = true,
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_command_palette(state);
        }
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('e') => {
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
                .cloned()
            else {
                return false;
            };
            let input = session.title.clone().unwrap_or_default();
            state.modal = Some(Modal::Rename {
                session_id: session.id,
                cursor: input.chars().count(),
                input,
            });
        }
        KeyCode::Char('d') => {
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
                .cloned()
            else {
                return false;
            };
            let title = session.title.clone().unwrap_or_else(|| session.id.clone());
            state.modal = Some(Modal::DeleteConfirm {
                session_id: session.id,
                title,
            });
        }
        KeyCode::Char('/') => state.sessions_filter_active = true,
        KeyCode::Down => {
            if !state.sessions_view.is_empty() {
                state.selected =
                    (state.selected + 1).min(state.sessions_view.len().saturating_sub(1));
            }
        }
        KeyCode::Up => {
            if state.selected > 0 {
                state.selected -= 1;
            }
        }
        KeyCode::PageDown => {
            if !state.sessions_view.is_empty() {
                state.selected =
                    (state.selected + 10).min(state.sessions_view.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => state.selected = state.selected.saturating_sub(10),
        KeyCode::Char('r') => {
            state.status = None;
            state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                state.status = Some(err.clone());
                push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                Vec::new()
            });
            sort_sessions(&mut state.sessions);
            state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
            state.selected = state
                .selected
                .min(state.sessions_view.len().saturating_sub(1));
        }
        KeyCode::Char('n') => {
            state.status = None;
            state.sessions_filter.clear();
            state.sessions_filter_active = false;
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(err) => {
                    state.status = Some(format!("failed to resolve cwd: {err}"));
                    return false;
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
                        state.status = Some(err.clone());
                        push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                        Vec::new()
                    });
                    sort_sessions(&mut state.sessions);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;

                    let messages = state
                        .backend
                        .load_messages(&session.id)
                        .unwrap_or_else(|err| {
                            state.status = Some(err.clone());
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
                }
                Err(err) => {
                    let msg = format!("failed to create session: {err}");
                    state.status = Some(msg.clone());
                    push_toast(state, ToastVariant::Error, msg, Duration::from_secs(4));
                }
            }
        }
        KeyCode::Char('f') => {
            state.status = None;
            state.sessions_filter.clear();
            state.sessions_filter_active = false;
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
            else {
                return false;
            };
            match state.backend.fork_session(&session.id, None) {
                Ok(forked) => {
                    state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                        state.status = Some(err.clone());
                        push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                        Vec::new()
                    });
                    sort_sessions(&mut state.sessions);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;

                    let messages = state
                        .backend
                        .load_messages(&forked.id)
                        .unwrap_or_else(|err| {
                            state.status = Some(err.clone());
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    state.screen = Screen::Chat(ChatState {
                        session: forked,
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
                }
                Err(err) => state.status = Some(format!("failed to fork session: {err}")),
            }
        }
        KeyCode::Enter => {
            state.status = None;
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
                .cloned()
            else {
                return false;
            };
            let messages = state
                .backend
                .load_messages(&session.id)
                .unwrap_or_else(|err| {
                    state.status = Some(err.clone());
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
        }
        _ => {}
    }
    false
}
