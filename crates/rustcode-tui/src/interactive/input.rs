use super::{
    build_prompt_history, build_transcript_lines, composer_backspace, composer_clear,
    composer_delete, composer_insert_str, composer_move_down, composer_move_end,
    composer_move_home, composer_move_left, composer_move_right, composer_move_up,
    compute_find_matches, compute_palette_view, compute_sessions_view, execute_command, find_next,
    find_prev, handle_slash_command, history_next, history_prev, maybe_execute_palette_query,
    open_command_palette, push_toast, refresh_chat_messages, set_find, sort_sessions,
    submit_prompt, transcript_area_height, ActivityItem, AppState, ChatFocus, ChatNav, ChatState,
    CreateSessionOptions, Duration, KeyCode, KeyEvent, KeyModifiers, Modal, Screen, ToastVariant,
};

mod input_chat;
mod input_sessions;

use self::input_chat::handle_chat_key;
use self::input_sessions::handle_sessions_key;

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    if let Some(pending) = state.pending_approval.take() {
        let decision = match key.code {
            KeyCode::Char('a') => Some(true),
            KeyCode::Char('d') => Some(false),
            KeyCode::Esc | KeyCode::Char('q') => Some(false),
            _ => None,
        };
        if let Some(decision) = decision {
            let _ = pending.reply.send(decision);
            state.status = Some(format!(
                "approval: tool={} decision={decision}",
                pending.request.tool
            ));
            push_toast(
                state,
                ToastVariant::Info,
                format!("approval: {} -> {decision}", pending.request.tool),
                Duration::from_secs(2),
            );
        } else {
            state.pending_approval = Some(pending);
        }
        return false;
    }

    if state.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => state.help_open = false,
            _ => {}
        }
        return false;
    }

    if state.modal.is_some() {
        handle_modal_key(state, key);
        return false;
    }

    match &state.screen {
        Screen::Sessions => handle_sessions_key(state, key),
        Screen::Chat(_) => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                return false;
            };

            let nav = handle_chat_key(state, &mut chat, key);
            match nav {
                ChatNav::Stay => state.screen = Screen::Chat(chat),
                ChatNav::ToSessions => state.screen = Screen::Sessions,
            }
            false
        }
    }
}

pub(super) fn handle_modal_key(state: &mut AppState, key: KeyEvent) {
    let Some(modal) = state.modal.take() else {
        return;
    };

    match modal {
        Modal::CommandPalette {
            mut query,
            mut selected,
            items,
            mut view,
        } => {
            match key.code {
                KeyCode::Esc => return,
                KeyCode::Backspace => {
                    query.pop();
                    view = compute_palette_view(&items, &query);
                    selected = selected.min(view.len().saturating_sub(1));
                }
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => {
                    if !view.is_empty() {
                        selected = (selected + 1).min(view.len().saturating_sub(1));
                    }
                }
                KeyCode::PageUp => selected = selected.saturating_sub(10),
                KeyCode::PageDown => {
                    if !view.is_empty() {
                        selected = (selected + 10).min(view.len().saturating_sub(1));
                    }
                }
                KeyCode::Enter => {
                    if maybe_execute_palette_query(state, &query) {
                        return;
                    }
                    let Some(idx) = view.get(selected).copied() else {
                        state.modal = Some(Modal::CommandPalette {
                            query,
                            selected,
                            items,
                            view,
                        });
                        return;
                    };
                    let Some(item) = items.get(idx) else {
                        return;
                    };
                    if !item.enabled {
                        let reason = item
                            .disabled_reason
                            .clone()
                            .unwrap_or_else(|| "Disabled".to_string());
                        push_toast(state, ToastVariant::Warning, reason, Duration::from_secs(3));
                        state.modal = Some(Modal::CommandPalette {
                            query,
                            selected,
                            items,
                            view,
                        });
                        return;
                    }
                    execute_command(state, item.id);
                    return;
                }
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        query.push(ch);
                        view = compute_palette_view(&items, &query);
                        selected = 0;
                    }
                }
                _ => {}
            }
            state.modal = Some(Modal::CommandPalette {
                query,
                selected,
                items,
                view,
            });
        }
        Modal::Search {
            mut query,
            current: _,
            matches: _,
        } => {
            let mut current = 0usize;
            let mut matches: Vec<usize> = Vec::new();
            let mut request_next = false;
            let mut request_prev = false;
            let mut update_query = false;
            match key.code {
                KeyCode::Esc => return,
                KeyCode::Enter | KeyCode::Char('n') | KeyCode::Down => request_next = true,
                KeyCode::Char('N') | KeyCode::Up => request_prev = true,
                KeyCode::Backspace => {
                    query.pop();
                    update_query = true;
                }
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        query.push(ch);
                        update_query = true;
                    }
                }
                _ => {}
            }

            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                state.screen = Screen::Sessions;
                return;
            };
            let viewport_h = transcript_area_height(state);

            if update_query {
                set_find(state, &mut chat, query.clone(), true, viewport_h);
            }
            if request_next {
                find_next(state, &mut chat, viewport_h);
            }
            if request_prev {
                find_prev(state, &mut chat, viewport_h);
            }

            if let Some(find) = &chat.find {
                matches = find.matches.clone();
                current = find.current;
                query = find.query.clone();
            }

            state.screen = Screen::Chat(chat);
            state.modal = Some(Modal::Search {
                query,
                current,
                matches,
            });
        }
        Modal::Rename {
            session_id,
            mut input,
            mut cursor,
        } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let next = if input.trim().is_empty() {
                    None
                } else {
                    Some(input.clone())
                };
                match state.backend.update_session_title(&session_id, next) {
                    Ok(_) => {
                        push_toast(
                            state,
                            ToastVariant::Success,
                            "renamed session",
                            Duration::from_secs(2),
                        );
                        state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                        sort_sessions(&mut state.sessions);
                        state.sessions_view =
                            compute_sessions_view(&state.sessions, &state.sessions_filter);
                        state.selected = state
                            .selected
                            .min(state.sessions_view.len().saturating_sub(1));
                    }
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to rename session: {err}"),
                        Duration::from_secs(4),
                    ),
                }
            }
            KeyCode::Left => {
                cursor = cursor.saturating_sub(1);
                state.modal = Some(Modal::Rename {
                    session_id,
                    input,
                    cursor,
                });
            }
            KeyCode::Right => {
                cursor = (cursor + 1).min(input.chars().count());
                state.modal = Some(Modal::Rename {
                    session_id,
                    input,
                    cursor,
                });
            }
            KeyCode::Backspace => {
                if cursor > 0 {
                    let mut out = String::new();
                    for (idx, ch) in input.chars().enumerate() {
                        if idx + 1 != cursor {
                            out.push(ch);
                        }
                    }
                    input = out;
                    cursor -= 1;
                }
                state.modal = Some(Modal::Rename {
                    session_id,
                    input,
                    cursor,
                });
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    let mut out = String::new();
                    let mut inserted = false;
                    for (idx, c) in input.chars().enumerate() {
                        if idx == cursor {
                            out.push(ch);
                            inserted = true;
                        }
                        out.push(c);
                    }
                    if !inserted {
                        out.push(ch);
                    }
                    input = out;
                    cursor += 1;
                }
                state.modal = Some(Modal::Rename {
                    session_id,
                    input,
                    cursor,
                });
            }
            _ => {
                state.modal = Some(Modal::Rename {
                    session_id,
                    input,
                    cursor,
                });
            }
        },
        Modal::DeleteConfirm { session_id, title } => match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {}
            KeyCode::Char('y') => match state.backend.delete_session(&session_id) {
                Ok(()) => {
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "deleted session",
                        Duration::from_secs(2),
                    );
                    state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                        push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                        Vec::new()
                    });
                    sort_sessions(&mut state.sessions);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = state
                        .selected
                        .min(state.sessions_view.len().saturating_sub(1));
                }
                Err(err) => push_toast(
                    state,
                    ToastVariant::Error,
                    format!("failed to delete session: {err}"),
                    Duration::from_secs(4),
                ),
            },
            _ => {
                state.modal = Some(Modal::DeleteConfirm { session_id, title });
            }
        },
    }
}
