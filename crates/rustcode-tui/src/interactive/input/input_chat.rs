use super::{
    build_prompt_history, composer_backspace, composer_clear, composer_delete, composer_insert_str,
    composer_kill_line_backward, composer_kill_line_forward, composer_move_down, composer_move_end,
    composer_move_home, composer_move_left, composer_move_right, composer_move_up,
    composer_word_left, composer_word_right, execute_command, filter_slash_commands,
    handle_slash_command, history_next, history_prev, open_command_palette, push_toast,
    refresh_chat_messages, submit_prompt, AppState, ChatFocus, ChatNav, ChatState, CommandId,
    CreateSessionOptions, Duration, KeyCode, KeyEvent, KeyModifiers, Modal, ToastVariant,
};

pub(super) fn handle_chat_key(
    state: &mut AppState,
    chat: &mut ChatState,
    key: KeyEvent,
) -> ChatNav {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    // ── SlashHelp popup navigation (Up/Down/Tab/Esc; Enter falls through) ────
    if matches!(&state.modal, Some(Modal::SlashHelp { .. })) && !ctrl && !alt {
        match key.code {
            KeyCode::Up => {
                if let Some(Modal::SlashHelp { selected, .. }) = &mut state.modal {
                    *selected = selected.saturating_sub(1);
                }
                return ChatNav::Stay;
            }
            KeyCode::Down => {
                if let Some(Modal::SlashHelp { query, selected }) = &mut state.modal {
                    let count = filter_slash_commands(query).len();
                    if count > 0 {
                        *selected = (*selected + 1).min(count - 1);
                    }
                }
                return ChatNav::Stay;
            }
            KeyCode::Tab => {
                complete_slash_selection(state, chat);
                return ChatNav::Stay;
            }
            KeyCode::Esc => {
                state.modal = None;
                return ChatNav::Stay;
            }
            KeyCode::Enter => {
                // Determine whether the selected command takes arguments.
                let has_args = if let Some(Modal::SlashHelp { query, selected }) = &state.modal {
                    filter_slash_commands(query)
                        .get(*selected)
                        .map(|(cmd, _)| cmd.trim_start_matches('/').contains(' '))
                } else {
                    None
                };
                match has_args {
                    Some(true) => {
                        // Command needs arguments (e.g. /find <query>) — complete
                        // with a trailing space so the user can type the argument,
                        // but do NOT execute yet.
                        complete_slash_selection(state, chat);
                        return ChatNav::Stay;
                    }
                    Some(false) => {
                        // Argument-free command — complete the composer text (e.g.
                        // "/help") and fall through so normal Enter executes it.
                        complete_slash_selection(state, chat);
                        // modal is now None; fall through to normal Enter handling.
                    }
                    None => {
                        // No match or no SlashHelp modal — close popup and fall through.
                        if matches!(state.modal, Some(Modal::SlashHelp { .. })) {
                            state.modal = None;
                        }
                    }
                }
                // fall through to normal Enter
            }
            _ => {} // fall through to normal key handling
        }
    }

    if alt && matches!(key.code, KeyCode::Tab) {
        chat.focus = next_focus(chat.focus, chat.activity_hidden);
        return ChatNav::Stay;
    }

    // ── Ctrl+key shortcuts (work regardless of focus) ───────────────
    if ctrl {
        match key.code {
            KeyCode::Char('c' | 'C') => {
                // Context-aware Ctrl+C:
                // First press clears input (if any) and shows "press again to exit"
                // Second press exits the application
                if !chat.composer_cleared_by_ctrl_c {
                    // First Ctrl+C - clear input (if any) and show hint
                    if !chat.composer.is_empty() {
                        composer_clear(chat);
                    }
                    chat.composer_cleared_by_ctrl_c = true;
                    push_toast(
                        state,
                        ToastVariant::Info,
                        "press Ctrl+C again to exit",
                        Duration::from_secs(3),
                    );
                    return ChatNav::Stay;
                }
                // Second Ctrl+C - exit the application
                return ChatNav::Exit;
            }
            KeyCode::Char('p' | 'P') => {
                open_command_palette(state, Some(chat));
                return ChatNav::Stay;
            }
            KeyCode::Char('n' | 'N') => {
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
                        let messages =
                            state
                                .backend
                                .load_messages(&session.id)
                                .unwrap_or_else(|err| {
                                    state.status = Some(err.clone());
                                    push_toast(
                                        state,
                                        ToastVariant::Error,
                                        err,
                                        Duration::from_secs(4),
                                    );
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
                        chat.tool_details = true;
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
            KeyCode::Char('f' | 'F') => {
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
                        let messages =
                            state
                                .backend
                                .load_messages(&forked.id)
                                .unwrap_or_else(|err| {
                                    state.status = Some(err.clone());
                                    push_toast(
                                        state,
                                        ToastVariant::Error,
                                        err,
                                        Duration::from_secs(4),
                                    );
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
                        chat.tool_details = true;
                        chat.find = None;
                        chat.running = None;
                        state.status = None;
                    }
                    Err(err) => state.status = Some(format!("failed to fork session: {err}")),
                }
                return ChatNav::Stay;
            }
            KeyCode::Char('t' | 'T') => {
                execute_command(state, CommandId::FileSearch);
                return ChatNav::Stay;
            }
            KeyCode::Char('s' | 'S') => {
                execute_command(state, CommandId::ToggleSkills);
                return ChatNav::Stay;
            }
            KeyCode::Char('b' | 'B') => {
                execute_command(state, CommandId::Feedback);
                return ChatNav::Stay;
            }
            KeyCode::Char('m' | 'M') => {
                execute_command(state, CommandId::SwitchModel);
                return ChatNav::Stay;
            }
            KeyCode::Char('a' | 'A') => {
                execute_command(state, CommandId::ManageProviders);
                return ChatNav::Stay;
            }
            // Ctrl+K: kill to end of current line
            KeyCode::Char('k' | 'K') => {
                if chat.focus == ChatFocus::Composer {
                    composer_kill_line_forward(chat);
                    return ChatNav::Stay;
                }
            }
            // Ctrl+U: kill to start of current line
            KeyCode::Char('u' | 'U') => {
                if chat.focus == ChatFocus::Composer {
                    composer_kill_line_backward(chat);
                    return ChatNav::Stay;
                }
            }
            // Ctrl+D: toggle tool call details (expand/collapse tool batches)
            KeyCode::Char('d' | 'D') => {
                chat.tool_details = !chat.tool_details;
                push_toast(
                    state,
                    ToastVariant::Info,
                    if chat.tool_details {
                        "tools: expanded"
                    } else {
                        "tools: collapsed"
                    },
                    Duration::from_secs(2),
                );
                return ChatNav::Stay;
            }
            // Ctrl+Y: toggle thinking/reasoning visibility
            KeyCode::Char('y' | 'Y') => {
                execute_command(state, CommandId::ToggleReasoning);
                return ChatNav::Stay;
            }
            // Ctrl+Left/Right: word jump
            KeyCode::Left => {
                if chat.focus == ChatFocus::Composer {
                    composer_word_left(chat);
                    return ChatNav::Stay;
                }
            }
            KeyCode::Right => {
                if chat.focus == ChatFocus::Composer {
                    composer_word_right(chat);
                    return ChatNav::Stay;
                }
            }
            KeyCode::Char('r' | 'R') => {
                refresh_chat_messages(state, chat);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "refreshed transcript",
                    Duration::from_secs(2),
                );
                return ChatNav::Stay;
            }
            KeyCode::Char('q' | 'Q') => {
                return ChatNav::ToSessions;
            }
            KeyCode::Char('w' | 'W') => {
                chat.activity_hidden = !chat.activity_hidden;
                if chat.activity_hidden && chat.focus == ChatFocus::Activity {
                    chat.focus = ChatFocus::Composer;
                }
                return ChatNav::Stay;
            }
            _ => {}
        }
    }

    // ── Activity details modal ───────────────────────────────────────
    if chat.details_open {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => chat.details_open = false,
            _ => {}
        }
        return ChatNav::Stay;
    }

    // ── Alt+key: prompt history navigation ──────────────────────────
    if chat.focus == ChatFocus::Composer && alt {
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

    // ── Shift+Enter: insert newline ────────────────────────────────────
    if chat.focus == ChatFocus::Composer
        && key.code == KeyCode::Enter
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        composer_insert_str(chat, "\n");
        return ChatNav::Stay;
    }

    // ── When the composer is focused, all plain character keys type ──
    // This must come before any single-key shortcut matches.
    if chat.focus == ChatFocus::Composer && !ctrl && !alt {
        if let KeyCode::Char(ch) = key.code {
            // Any manual typing invalidates a pending large-paste buffer.
            chat.paste_buffer = None;
            composer_insert_str(chat, &ch.to_string());
            return ChatNav::Stay;
        }
    }

    // ── Non-character keys and shortcuts ────────────────────────────
    match key.code {
        KeyCode::Char('?') => state.help_open = true,
        KeyCode::Esc => {
            if chat.focus != ChatFocus::Composer {
                chat.focus = ChatFocus::Composer;
                return ChatNav::Stay;
            }
            // Escape focuses composer; if already focused, clear composer or show hint.
            // Does NOT navigate to sessions - use Ctrl+Q for that.
            if !chat.composer.is_empty() {
                composer_clear(chat);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "composer cleared",
                    Duration::from_secs(2),
                );
            }
        }
        // "/" from non-composer focus: move to composer and insert "/" so user can type /commands
        KeyCode::Char('/') => {
            if chat.focus != ChatFocus::Composer {
                chat.focus = ChatFocus::Composer;
                composer_insert_str(chat, "/");
                return ChatNav::Stay;
            }
        }
        // Scroll / selection
        KeyCode::PageUp => {
            chat.scroll = chat.scroll.saturating_add(5);
        }
        KeyCode::PageDown => {
            chat.scroll = chat.scroll.saturating_sub(5);
        }
        KeyCode::Down => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.activity_selected =
                        (chat.activity_selected + 1).min(chat.activity.len().saturating_sub(1));
                }
            } else if chat.focus == ChatFocus::Composer {
                composer_move_down(chat);
            }
        }
        KeyCode::Up => {
            if chat.focus == ChatFocus::Activity {
                chat.activity_selected = chat.activity_selected.saturating_sub(1);
            } else if chat.focus == ChatFocus::Composer {
                composer_move_up(chat);
            }
        }
        KeyCode::Backspace => {
            if chat.focus == ChatFocus::Composer {
                chat.paste_buffer = None;
                composer_backspace(chat);
            }
        }
        KeyCode::Delete => {
            if chat.focus == ChatFocus::Composer {
                chat.paste_buffer = None;
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
            }
        }
        KeyCode::End => {
            if chat.focus == ChatFocus::Composer {
                composer_move_end(chat);
            }
        }
        KeyCode::Enter => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.details_open = true;
                }
                return ChatNav::Stay;
            }

            if chat.running.is_some() {
                return ChatNav::Stay;
            }
            // Use the full paste_buffer text if present (large paste shown as summary).
            let prompt = chat
                .paste_buffer
                .take()
                .unwrap_or_else(|| chat.composer.trim().to_string());
            if prompt.is_empty() {
                // Empty Enter with an error status → show full error detail
                if let Some(msg) = state.status.clone() {
                    state.modal = Some(Modal::ErrorDetail { message: msg });
                }
                return ChatNav::Stay;
            }
            if prompt.starts_with('/') {
                composer_clear(chat);
                return handle_slash_command(state, chat, &prompt);
            }
            composer_clear(chat);
            submit_prompt(state, chat, prompt);
        }
        _ => {}
    }

    ChatNav::Stay
}

fn next_focus(focus: ChatFocus, activity_hidden: bool) -> ChatFocus {
    match focus {
        ChatFocus::Composer => {
            if activity_hidden {
                ChatFocus::Composer // no-op when activity panel is hidden
            } else {
                ChatFocus::Activity
            }
        }
        ChatFocus::Activity => ChatFocus::Composer,
    }
}

/// Replace the composer content with the currently selected slash command and
/// close the [`Modal::SlashHelp`] popup.
///
/// For commands that take arguments (contain a space in the table entry, e.g.
/// `/find <query>`), the composer is populated with `/cmd ` (trailing space)
/// so the user can type the argument immediately.  For argument-free commands
/// the full command text is inserted.
fn complete_slash_selection(state: &mut AppState, chat: &mut ChatState) {
    let completion = if let Some(Modal::SlashHelp { query, selected }) = &state.modal {
        let filtered = filter_slash_commands(query);
        filtered.get(*selected).map(|(cmd, _)| {
            // Strip the leading `/`, split on whitespace to get the bare name.
            let bare = cmd.trim_start_matches('/');
            if bare.contains(' ') {
                // Command takes arguments — insert `/cmd ` with trailing space.
                let stem = bare.split_whitespace().next().unwrap_or(bare);
                format!("/{stem} ")
            } else {
                format!("/{bare}")
            }
        })
    } else {
        None
    };

    if let Some(text) = completion {
        // Replace composer with the completed command.
        let byte_len = text.len();
        chat.composer = text;
        chat.composer_cursor = byte_len;
        state.modal = None;
    }
}
