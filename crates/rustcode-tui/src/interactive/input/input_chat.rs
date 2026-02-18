use super::{AppState, ChatState, KeyEvent, ChatNav, KeyModifiers, KeyCode, ActivityItem, open_command_palette, push_toast, ToastVariant, Duration, CreateSessionOptions, composer_clear, build_prompt_history, ChatFocus, history_prev, history_next, build_transcript_lines, compute_find_matches, Modal, refresh_chat_messages, find_next, transcript_area_height, find_prev, composer_move_down, composer_move_up, composer_backspace, composer_delete, composer_move_left, composer_move_right, composer_move_home, composer_move_end, composer_insert_str, handle_slash_command, submit_prompt};

pub(super) fn handle_chat_key(
    state: &mut AppState,
    chat: &mut ChatState,
    key: KeyEvent,
) -> ChatNav {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        if let Some(running) = &chat.running {
            running.cancellation.cancel();
            chat.activity.push(ActivityItem::Warning {
                message: "cancel requested".to_string(),
            });
        }
        return ChatNav::Stay;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('p')) {
        open_command_palette(state);
        return ChatNav::Stay;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('n')) {
        if chat.running.is_some() {
            state.status = Some("cannot create session while running".to_string());
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
                state.status = Some(format!("failed to resolve cwd: {err}"));
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
                        state.status = Some(err.clone());
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
                chat.find = None;
                chat.running = None;
                state.status = None;
            }
            Err(err) => {
                let msg = format!("failed to create session: {err}");
                state.status = Some(msg.clone());
                push_toast(state, ToastVariant::Error, msg, Duration::from_secs(4));
            }
        }
        return ChatNav::Stay;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('f')) {
        if chat.running.is_some() {
            state.status = Some("cannot fork session while running".to_string());
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
                        state.status = Some(err.clone());
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
                chat.find = None;
                chat.running = None;
                state.status = None;
            }
            Err(err) => state.status = Some(format!("failed to fork session: {err}")),
        }
        return ChatNav::Stay;
    }

    if chat.details_open {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => chat.details_open = false,
            _ => {}
        }
        return ChatNav::Stay;
    }

    if chat.focus == ChatFocus::Composer && key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Up => {
                history_prev(chat);
                return ChatNav::Stay;
            }
            KeyCode::Down => {
                history_next(chat);
                return ChatNav::Stay;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Tab => {
            chat.focus = match chat.focus {
                ChatFocus::Composer => ChatFocus::Transcript,
                ChatFocus::Transcript => ChatFocus::Activity,
                ChatFocus::Activity => ChatFocus::Composer,
            };
        }
        KeyCode::Char('/') => {
            if chat.focus == ChatFocus::Transcript {
                let query = chat
                    .find
                    .as_ref()
                    .map(|find| find.query.clone())
                    .unwrap_or_default();
                let transcript = build_transcript_lines(chat);
                let matches = compute_find_matches(&transcript, &query);
                state.modal = Some(Modal::Search {
                    query,
                    current: chat.find.as_ref().map_or(0, |f| f.current),
                    matches,
                });
                return ChatNav::Stay;
            }
        }
        KeyCode::Char('?') => state.help_open = true,
        KeyCode::Char('t') => chat.tool_details = !chat.tool_details,
        KeyCode::Esc => {
            if chat.composer.is_empty() {
                return ChatNav::ToSessions;
            }
            composer_clear(chat);
        }
        KeyCode::Char('q') => return ChatNav::ToSessions,
        KeyCode::Char('r') => refresh_chat_messages(state, chat),
        KeyCode::Char('n') => {
            if chat.focus == ChatFocus::Transcript {
                find_next(state, chat, transcript_area_height(state));
                return ChatNav::Stay;
            }
        }
        KeyCode::Char('N') => {
            if chat.focus == ChatFocus::Transcript {
                find_prev(state, chat, transcript_area_height(state));
                return ChatNav::Stay;
            }
        }
        KeyCode::PageUp => {
            if chat.focus == ChatFocus::Activity {
                chat.activity_selected = chat.activity_selected.saturating_sub(10);
            } else if chat.focus != ChatFocus::Composer {
                chat.scroll = chat.scroll.saturating_add(5);
            }
        }
        KeyCode::PageDown => {
            if chat.focus == ChatFocus::Activity {
                chat.activity_selected =
                    (chat.activity_selected + 10).min(chat.activity.len().saturating_sub(1));
            } else if chat.focus != ChatFocus::Composer {
                chat.scroll = chat.scroll.saturating_sub(5);
            }
        }
        KeyCode::Down => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.activity_selected =
                        (chat.activity_selected + 1).min(chat.activity.len().saturating_sub(1));
                }
            } else if chat.focus == ChatFocus::Composer {
                composer_move_down(chat);
            } else {
                chat.scroll = chat.scroll.saturating_sub(1);
            }
        }
        KeyCode::Up => {
            if chat.focus == ChatFocus::Activity {
                chat.activity_selected = chat.activity_selected.saturating_sub(1);
            } else if chat.focus == ChatFocus::Composer {
                composer_move_up(chat);
            } else {
                chat.scroll = chat.scroll.saturating_add(1);
            }
        }
        KeyCode::Backspace => {
            if chat.focus == ChatFocus::Composer {
                composer_backspace(chat);
            }
        }
        KeyCode::Delete => {
            if chat.focus == ChatFocus::Composer {
                composer_delete(chat);
            }
        }
        KeyCode::Left => {
            if chat.focus == ChatFocus::Composer {
                composer_move_left(chat);
            }
        }
        KeyCode::Right => {
            if chat.focus == ChatFocus::Composer {
                composer_move_right(chat);
            }
        }
        KeyCode::Home => {
            if chat.focus == ChatFocus::Composer {
                composer_move_home(chat);
            } else if chat.focus == ChatFocus::Transcript {
                chat.scroll = u16::MAX;
            }
        }
        KeyCode::End => {
            if chat.focus == ChatFocus::Composer {
                composer_move_end(chat);
            } else if chat.focus == ChatFocus::Transcript {
                chat.scroll = 0;
            }
        }
        KeyCode::Enter => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.details_open = true;
                }
                return ChatNav::Stay;
            }

            if chat.focus == ChatFocus::Composer && key.modifiers.contains(KeyModifiers::ALT) {
                composer_insert_str(chat, "\n");
                return ChatNav::Stay;
            }

            if chat.focus == ChatFocus::Transcript {
                find_next(state, chat, transcript_area_height(state));
                return ChatNav::Stay;
            }

            if chat.running.is_some() {
                return ChatNav::Stay;
            }
            let prompt = chat.composer.trim().to_string();
            if prompt.is_empty() {
                return ChatNav::Stay;
            }
            if prompt.starts_with('/') {
                composer_clear(chat);
                return handle_slash_command(state, chat, &prompt);
            }
            composer_clear(chat);
            submit_prompt(state, chat, prompt);
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
                && chat.focus == ChatFocus::Composer
            {
                composer_insert_str(chat, &ch.to_string());
            }
        }
        _ => {}
    }

    ChatNav::Stay
}
