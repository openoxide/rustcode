use super::{
    build_prompt_history, build_provider_entries, composer_backspace, composer_clear,
    composer_delete, composer_insert_str, composer_kill_line_backward, composer_kill_line_forward,
    composer_move_down, composer_move_end, composer_move_home, composer_move_left,
    composer_move_right, composer_move_up, composer_word_left, composer_word_right,
    compute_palette_view, compute_sessions_view, execute_command, filter_files, filter_models,
    filter_provider_entries, find_next, find_prev, handle_slash_command, history_next,
    history_prev, maybe_execute_palette_query, open_command_palette, provider_connect_methods,
    provider_display_name, provider_env_hint, push_toast, refresh_chat_messages, set_find,
    sort_sessions, submit_prompt, transcript_area_height, AppState, ApprovalResponse, Arc,
    ChatFocus, ChatNav, ChatState, CommandId, ConnectMethod, CreateSessionOptions, Duration,
    KeyCode, KeyEvent, KeyModifiers, Modal, ProviderManagerStep, ProviderOAuthDone,
    ProviderOAuthStarted, Screen, ToastVariant, SLASH_COMMANDS,
};

mod input_chat;
mod input_sessions;

use self::input_chat::handle_chat_key;
use self::input_sessions::handle_sessions_key;

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    if state.pending_approval.is_some() {
        let perm = state
            .pending_approval
            .as_ref()
            .unwrap()
            .request
            .permission
            .to_lowercase();
        let is_edit = perm == "write" || perm == "edit";
        let is_cmd = perm == "exec";
        let options_count: usize = if is_edit || is_cmd { 3 } else { 2 };

        match key.code {
            KeyCode::Left | KeyCode::Up => {
                if state.approval_selection > 0 {
                    state.approval_selection -= 1;
                }
            }
            KeyCode::Right | KeyCode::Down => {
                if state.approval_selection + 1 < options_count {
                    state.approval_selection += 1;
                }
            }
            KeyCode::Enter => {
                let pending = state.pending_approval.take().unwrap();
                let response = match state.approval_selection {
                    0 => ApprovalResponse::AllowOnce,
                    1 if is_edit => ApprovalResponse::AllowAllEdits,
                    1 if is_cmd => ApprovalResponse::AllowAllCommands,
                    _ => ApprovalResponse::Deny,
                };
                let label = match response {
                    ApprovalResponse::AllowOnce => "approved once",
                    ApprovalResponse::AllowAllEdits => "approved all edits",
                    ApprovalResponse::AllowAllCommands => "approved all commands",
                    ApprovalResponse::Deny => "denied",
                };
                push_toast(
                    state,
                    ToastVariant::Info,
                    format!("{}: {label}", pending.request.tool),
                    Duration::from_secs(2),
                );
                let _ = pending.reply.send(response);
                state.approval_selection = 0;
            }
            KeyCode::Esc => {
                let pending = state.pending_approval.take().unwrap();
                push_toast(
                    state,
                    ToastVariant::Info,
                    format!("{}: denied", pending.request.tool),
                    Duration::from_secs(2),
                );
                let _ = pending.reply.send(ApprovalResponse::Deny);
                state.approval_selection = 0;
            }
            _ => {} // all other keys: selector stays, no action
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
        // SlashHelp is a transparent popup — keys still reach handle_chat_key so
        // the user can keep typing while the suggestion list is visible.
        if !matches!(&state.modal, Some(Modal::SlashHelp { .. })) {
            handle_modal_key(state, key);
            return false;
        }
    }

    // Global: Ctrl+E shows full error details from any screen/focus
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('e' | 'E'))
    {
        if let Some(msg) = state.status.clone() {
            state.modal = Some(Modal::ErrorDetail { message: msg });
            return false;
        }
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
                ChatNav::Stay => {
                    sync_slash_help(state, &chat);
                    state.screen = Screen::Chat(chat);
                    false
                }
                ChatNav::ToSessions => {
                    state.screen = Screen::Sessions;
                    false
                }
                ChatNav::Exit => true,
            }
        }
    }
}

/// Returns the subset of [`SLASH_COMMANDS`] whose command name contains `query`
/// (case-insensitive prefix/substring match).
pub(super) fn filter_slash_commands(query: &str) -> Vec<(&'static str, &'static str)> {
    if query.is_empty() {
        return SLASH_COMMANDS.to_vec();
    }
    let needle = query.to_ascii_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| {
            // Match on the part after '/' before any space (the command stem)
            let stem = cmd.trim_start_matches('/');
            let stem = stem.split_whitespace().next().unwrap_or(stem);
            stem.to_ascii_lowercase().contains(&needle)
        })
        .copied()
        .collect()
}

/// Sync the [`Modal::SlashHelp`] popup with the current composer content.
///
/// Called after every key event while in chat screen.  Opens the popup when
/// the composer starts with `/` on a single line; closes it otherwise.
fn sync_slash_help(state: &mut AppState, chat: &ChatState) {
    if chat.composer.starts_with('/') && !chat.composer.contains('\n') {
        let query = chat.composer.trim_start_matches('/').to_string();
        match &mut state.modal {
            Some(Modal::SlashHelp { query: q, .. }) => *q = query,
            _ => {
                state.modal = Some(Modal::SlashHelp {
                    query,
                    selected: 0,
                })
            }
        }
    } else if matches!(state.modal, Some(Modal::SlashHelp { .. })) {
        state.modal = None;
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
            mut query,
            mut view,
            mut selected,
            current_model,
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
                    if let Some(model_id) = view.get(selected).and_then(|idx| entries.get(*idx)) {
                        let new_model = model_id.clone();

                        // 1. Update defaults so new sessions and footer fallback use new model
                        state.defaults.model = new_model.clone();
                        if let Some((provider, _)) = new_model.split_once('/') {
                            state.defaults.provider = provider.to_string();
                        }

                        // 2. Replace state.config so the next submit_prompt uses the new model.
                        //    Also clear llm_provider so the model prefix ("openai/gpt-4o")
                        //    drives provider selection instead of any explicit override.
                        if let Some(config) = &state.config {
                            let mut updated = (**config).clone();
                            updated.model = new_model.clone();
                            updated.llm_provider = String::new();
                            // Rebuild the LLM client for the new provider so the engine
                            // routes requests to the correct API endpoint immediately.
                            if let Some(cell) = &state.llm_cell {
                                match rustcode_llm::build_client(&updated) {
                                    Ok(new_client) => {
                                        *cell.write().expect("llm cell lock not poisoned") =
                                            new_client;
                                    }
                                    Err(err) => {
                                        push_toast(
                                            state,
                                            ToastVariant::Warning,
                                            format!("llm client: {err}"),
                                            Duration::from_secs(4),
                                        );
                                    }
                                }
                            }
                            state.config = Some(Arc::new(updated));
                        }

                        // 3. Update current session model so footer reflects change immediately
                        if let Screen::Chat(ref mut chat) = state.screen {
                            chat.session.model = new_model.clone();
                        }

                        push_toast(
                            state,
                            ToastVariant::Success,
                            format!("model: {new_model}"),
                            Duration::from_secs(3),
                        );
                    }
                    return;
                }
                KeyCode::Backspace => {
                    query.pop();
                    view = filter_models(&entries, &query);
                    selected = 0;
                }
                KeyCode::Char(ch) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT)
                    {
                        query.push(ch);
                        view = filter_models(&entries, &query);
                        selected = 0;
                    }
                }
                _ => {}
            }
            state.modal = Some(Modal::ModelSelect {
                entries,
                query,
                view,
                selected,
                current_model,
            });
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

/// Handle a key event when the provider manager modal is open.
fn handle_provider_manager_key(state: &mut AppState, step: ProviderManagerStep, key: KeyEvent) {
    match step {
        ProviderManagerStep::List {
            entries,
            mut query,
            mut view,
            mut selected,
        } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Down => {
                if !view.is_empty() {
                    selected = (selected + 1).min(view.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::PageUp => {
                selected = selected.saturating_sub(10);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::PageDown => {
                if !view.is_empty() {
                    selected = (selected + 10).min(view.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Enter => {
                let entry = view
                    .get(selected)
                    .and_then(|idx| entries.get(*idx))
                    .cloned();
                if let Some(entry) = entry {
                    let methods = provider_connect_methods(&entry.provider_id, entry.connected);
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::MethodSelect {
                            provider_id: entry.provider_id,
                            display_name: entry.display_name,
                            methods,
                            selected: 0,
                        },
                    });
                } else {
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::List {
                            entries,
                            query,
                            view,
                            selected,
                        },
                    });
                }
            }
            KeyCode::Backspace => {
                query.pop();
                view = filter_provider_entries(&entries, &query);
                selected = 0;
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    query.push(ch);
                    view = filter_provider_entries(&entries, &query);
                    selected = 0;
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
        },

        ProviderManagerStep::MethodSelect {
            provider_id,
            display_name,
            methods,
            mut selected,
        } => match key.code {
            KeyCode::Esc => {
                // Go back to list
                let entries = build_provider_entries();
                let view = filter_provider_entries(&entries, "");
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query: String::new(),
                        view,
                        selected: 0,
                    },
                });
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
            KeyCode::Down => {
                if !methods.is_empty() {
                    selected = (selected + 1).min(methods.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
            KeyCode::Enter => {
                let method = methods.get(selected).copied();
                match method {
                    Some(ConnectMethod::ApiKey) => {
                        let env_hint = provider_env_hint(&provider_id);
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::ApiKeyInput {
                                provider_id,
                                display_name,
                                env_hint,
                                input: String::new(),
                                cursor: 0,
                            },
                        });
                    }
                    Some(ConnectMethod::OAuthDeviceCode) => {
                        // Spawn background task for device code flow
                        let (start_tx, start_rx) =
                            std::sync::mpsc::channel::<Result<ProviderOAuthStarted, String>>();
                        let (done_tx, done_rx) =
                            std::sync::mpsc::channel::<Result<ProviderOAuthDone, String>>();
                        state.provider_oauth_start_rx = Some(start_rx);
                        state.provider_oauth_done_rx = Some(done_rx);

                        let provider_id_clone = provider_id.clone();
                        state.runtime.spawn(async move {
                            let flow = match rustcode_auth::start_device_code_flow(
                                &provider_id_clone,
                                None,
                            )
                            .await
                            {
                                Ok(f) => f,
                                Err(err) => {
                                    let _ = start_tx.send(Err(err.to_string()));
                                    return;
                                }
                            };

                            let _ = start_tx.send(Ok(ProviderOAuthStarted {
                                provider_id: provider_id_clone.clone(),
                                verification_uri: flow.verification_uri.clone(),
                                user_code: flow.user_code.clone(),
                            }));

                            let timeout = std::time::Duration::from_secs(flow.expires_in_secs);
                            let cred = match rustcode_auth::poll_device_code_flow_for_credential(
                                &flow, timeout,
                            )
                            .await
                            {
                                Ok(c) => c,
                                Err(err) => {
                                    let _ = done_tx.send(Err(err.to_string()));
                                    return;
                                }
                            };

                            let now_unix = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            let expires_at_unix =
                                cred.expires_in_secs.map(|secs| now_unix + secs as i64);

                            let _ = done_tx.send(Ok(ProviderOAuthDone {
                                provider_id: provider_id_clone,
                                access_token: cred.access_token,
                                refresh_token: cred.refresh_token,
                                expires_at_unix,
                                account_id: cred.account_id,
                            }));
                        });

                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::OAuthStarting {
                                provider_id,
                                display_name,
                            },
                        });
                    }
                    Some(ConnectMethod::Disconnect) => {
                        let store = rustcode_auth::AuthStore::open_default();
                        match store.remove(&provider_id) {
                            Ok(_) => push_toast(
                                state,
                                ToastVariant::Success,
                                format!("{display_name} disconnected"),
                                Duration::from_secs(3),
                            ),
                            Err(err) => push_toast(
                                state,
                                ToastVariant::Error,
                                format!("failed to disconnect: {err}"),
                                Duration::from_secs(4),
                            ),
                        }
                        // Return to refreshed list
                        let entries = build_provider_entries();
                        let view = filter_provider_entries(&entries, "");
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::List {
                                entries,
                                query: String::new(),
                                view,
                                selected: 0,
                            },
                        });
                    }
                    None => {
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::MethodSelect {
                                provider_id,
                                display_name,
                                methods,
                                selected,
                            },
                        });
                    }
                }
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
        },

        ProviderManagerStep::ApiKeyInput {
            provider_id,
            display_name,
            env_hint,
            mut input,
            mut cursor,
        } => match key.code {
            KeyCode::Esc => {
                // Go back to method select
                let methods = provider_connect_methods(&provider_id, false);
                let display = provider_display_name(&provider_id);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name: display,
                        methods,
                        selected: 0,
                    },
                });
            }
            KeyCode::Enter => {
                if input.trim().is_empty() {
                    push_toast(
                        state,
                        ToastVariant::Warning,
                        "API key must not be empty",
                        Duration::from_secs(3),
                    );
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::ApiKeyInput {
                            provider_id,
                            display_name,
                            env_hint,
                            input,
                            cursor,
                        },
                    });
                } else {
                    let store = rustcode_auth::AuthStore::open_default();
                    match store.set_api_key(&provider_id, input.trim()) {
                        Ok(()) => {
                            push_toast(
                                state,
                                ToastVariant::Success,
                                format!("{display_name} connected!"),
                                Duration::from_secs(4),
                            );
                            // Close modal and re-open refreshed list
                            let entries = build_provider_entries();
                            let view = filter_provider_entries(&entries, "");
                            state.modal = Some(Modal::ProviderManager {
                                step: ProviderManagerStep::List {
                                    entries,
                                    query: String::new(),
                                    view,
                                    selected: 0,
                                },
                            });
                        }
                        Err(err) => {
                            push_toast(
                                state,
                                ToastVariant::Error,
                                format!("failed to save key: {err}"),
                                Duration::from_secs(5),
                            );
                            state.modal = Some(Modal::ProviderManager {
                                step: ProviderManagerStep::ApiKeyInput {
                                    provider_id,
                                    display_name,
                                    env_hint,
                                    input,
                                    cursor,
                                },
                            });
                        }
                    }
                }
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
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            KeyCode::Left => {
                cursor = cursor.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            KeyCode::Right => {
                cursor = (cursor + 1).min(input.chars().count());
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
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
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
        },

        // OAuthStarting / OAuthPending — only Esc cancels.
        ProviderManagerStep::OAuthStarting { .. } | ProviderManagerStep::OAuthPending { .. } => {
            if matches!(key.code, KeyCode::Esc) {
                // Drop channels by removing from AppState
                state.provider_oauth_start_rx = None;
                state.provider_oauth_done_rx = None;
                push_toast(
                    state,
                    ToastVariant::Info,
                    "OAuth cancelled",
                    Duration::from_secs(2),
                );
            } else {
                state.modal = Some(Modal::ProviderManager { step });
            }
        }
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
