use super::input_pm::handle_provider_manager_key;
use super::input_provider::handle_model_select_key;
use super::{
    composer_insert_str, compute_palette_view, compute_sessions_view, execute_command,
    filter_files, find_next, find_prev, maybe_execute_palette_query, push_toast, set_find,
    sort_sessions, transcript_area_height, AppState, Duration, KeyCode, KeyEvent, KeyModifiers,
    Modal, Screen, ToastVariant,
};

/// Handle a key event when a modal overlay is open.
///
/// Returns immediately (leaving `state.modal = None`) to dismiss the modal on
/// Escape, or restores `state.modal` before returning when the modal should
/// stay open.
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
        Modal::FileSearch {
            mut query,
            entries,
            mut view,
            mut selected,
        } => {
            match key.code {
                KeyCode::Esc => return,
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !view.is_empty() {
                        selected = (selected + 1).min(view.len().saturating_sub(1));
                    }
                }
                KeyCode::PageUp => {
                    selected = selected.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    if !view.is_empty() {
                        selected = (selected + 10).min(view.len().saturating_sub(1));
                    }
                }
                KeyCode::Enter => {
                    if let Some(path) = view.get(selected).and_then(|idx| entries.get(*idx)) {
                        let insert = format!("@{path}");
                        let Screen::Chat(mut chat) =
                            std::mem::replace(&mut state.screen, Screen::Sessions)
                        else {
                            return;
                        };
                        composer_insert_str(&mut chat, &insert);
                        state.screen = Screen::Chat(chat);
                        push_toast(
                            state,
                            ToastVariant::Info,
                            format!("inserted: {path}"),
                            Duration::from_secs(2),
                        );
                    }
                    return;
                }
                KeyCode::Backspace => {
                    query.pop();
                    view = filter_files(&entries, &query);
                    selected = 0;
                }
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        query.push(ch);
                        view = filter_files(&entries, &query);
                        selected = 0;
                    }
                }
                _ => {}
            }
            state.modal = Some(Modal::FileSearch {
                query,
                entries,
                view,
                selected,
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
        Modal::ErrorDetail { .. } => {
            // Any key dismisses the error detail popup
        }
        Modal::SlashHelp { .. } => {
            // SlashHelp is transparent — handle_chat_key handles all keys.
            // This arm is unreachable in normal flow (see input.rs handle_key).
        }
        Modal::SkillToggle {
            mut skills,
            mut selected,
        } => {
            match key.code {
                KeyCode::Esc => return,
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !skills.is_empty() {
                        selected = (selected + 1).min(skills.len().saturating_sub(1));
                    }
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(entry) = skills.get_mut(selected) {
                        entry.2 = !entry.2;
                        let new_enabled = entry.2;
                        let name = entry.0.clone();
                        if let Some(skill) =
                            state.defaults.skills.iter_mut().find(|s| s.name() == name)
                        {
                            skill.metadata.enabled = new_enabled;
                        }
                    }
                }
                _ => {}
            }
            if !skills.is_empty() {
                state.modal = Some(Modal::SkillToggle { skills, selected });
            }
        }
        Modal::Feedback {
            mut rating,
            mut comment,
            mut comment_active,
        } => {
            let mut close = false;
            let no_mod = !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT);

            // Shared keys regardless of focus
            match key.code {
                KeyCode::Tab => comment_active = !comment_active,
                KeyCode::Enter => match rating {
                    None => push_toast(
                        state,
                        ToastVariant::Warning,
                        "select a rating first: u = thumbs up, d = thumbs down",
                        Duration::from_secs(3),
                    ),
                    Some(is_positive) => {
                        write_feedback(is_positive, &comment);
                        push_toast(
                            state,
                            ToastVariant::Success,
                            "feedback recorded — thank you!",
                            Duration::from_secs(3),
                        );
                        close = true;
                    }
                },
                _ => {}
            }

            if !close {
                if comment_active {
                    // Comment field focus
                    match key.code {
                        KeyCode::Esc => comment_active = false,
                        KeyCode::Backspace => {
                            comment.pop();
                        }
                        KeyCode::Char(ch) if no_mod => comment.push(ch),
                        _ => {}
                    }
                } else {
                    // Rating button focus
                    match key.code {
                        KeyCode::Esc => close = true,
                        KeyCode::Char('u' | 'U' | '+') | KeyCode::Left => {
                            rating = Some(true);
                        }
                        KeyCode::Char('d' | 'D' | '-') | KeyCode::Right => {
                            rating = Some(false);
                        }
                        _ => {}
                    }
                }
            }

            if !close {
                state.modal = Some(Modal::Feedback {
                    rating,
                    comment,
                    comment_active,
                });
            }
        }
        Modal::ModelSelect {
            entries,
            query,
            view,
            selected,
            current_model,
        } => {
            handle_model_select_key(state, entries, query, view, selected, current_model, key);
        }
        Modal::ProviderManager { step } => {
            handle_provider_manager_key(state, step, key);
        }
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

/// Append a feedback entry to `~/.local/share/rustcode/feedback.jsonl`.
///
/// Failures are silently ignored so bad writes don't disrupt the TUI.
fn write_feedback(is_positive: bool, comment: &str) {
    use std::io::Write as _;
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = std::path::PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("rustcode")
        .join("feedback.jsonl");

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let rating = if is_positive { "up" } else { "down" };
    // Escape double-quotes in comment for safe inline JSON
    let safe_comment = comment.trim().replace('\\', "\\\\").replace('"', "\\\"");
    let entry = format!(r#"{{"ts":{ts},"rating":"{rating}","comment":"{safe_comment}"}}"#);

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{entry}");
    }
}
