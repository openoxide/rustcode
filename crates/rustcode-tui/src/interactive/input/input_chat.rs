use std::sync::atomic::Ordering;

use super::super::ApprovalMode;
use super::{
    build_prompt_history, composer_backspace, composer_clear, composer_delete, composer_insert_str,
    composer_kill_line_backward, composer_kill_line_forward, composer_move_down, composer_move_end,
    composer_move_home, composer_move_left, composer_move_right, composer_move_up,
    composer_word_left, composer_word_right, execute_command, filter_slash_commands,
    handle_slash_command, history_next, history_prev, open_command_palette, push_toast,
    refresh_chat_messages, submit_prompt, AppState, ChatFocus, ChatNav, ChatState, CommandId,
    Duration, KeyCode, KeyEvent, KeyModifiers, Modal, ToastVariant,
};

#[path = "input_chat_helpers.rs"]
mod input_chat_helpers;

use self::input_chat_helpers::{
    complete_slash_selection, consume_large_paste_summary_with_backspace,
    consume_large_paste_summary_with_delete,
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

    // Shift+Tab (BackTab): cycle approval mode.
    if matches!(key.code, KeyCode::BackTab) {
        cycle_approval_mode(state, Some(chat));
        return ChatNav::Stay;
    }

    let is_interrupt_key =
        (ctrl && matches!(key.code, KeyCode::Char('c' | 'C'))) || matches!(key.code, KeyCode::Esc);
    if !is_interrupt_key {
        chat.composer_cleared_by_ctrl_c = false;
    }

    // ── Ctrl+key shortcuts (work regardless of focus) ───────────────
    if ctrl {
        match key.code {
            KeyCode::Char('c' | 'C') => {
                return handle_double_interrupt_key(state, chat, "Ctrl+C");
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
                chat.session = crate::new_draft_session(
                    None,
                    cwd,
                    state.defaults.workspace_root.clone(),
                    state.defaults.model.clone(),
                );
                chat.messages = Vec::new();
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
                chat.output_details = false;
                chat.find = None;
                chat.running = None;
                state.status = None;
                push_toast(
                    state,
                    ToastVariant::Success,
                    "new session (saved on first message)",
                    Duration::from_secs(2),
                );
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
                        chat.tool_details = false;
                        chat.output_details = false;
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
            // Ctrl+O: toggle tool details (summary vs expanded).
            KeyCode::Char('o' | 'O') => {
                let expand = !chat.tool_details;
                chat.tool_details = expand;
                chat.output_details = expand;
                chat.show_reasoning = expand;
                push_toast(
                    state,
                    ToastVariant::Info,
                    if expand {
                        "details: expanded"
                    } else {
                        "details: collapsed"
                    },
                    Duration::from_secs(2),
                );
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
            return handle_double_interrupt_key(state, chat, "Esc");
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
            let max = chat.last_max_scroll.get();
            if max == 0 {
                chat.scroll = 0;
            } else {
                chat.scroll = chat.scroll.min(max).saturating_add(5).min(max);
            }
        }
        KeyCode::PageDown => {
            let max = chat.last_max_scroll.get();
            chat.scroll = chat.scroll.min(max).saturating_sub(5);
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
                if consume_large_paste_summary_with_backspace(
                    &mut chat.composer,
                    &mut chat.composer_cursor,
                    &mut chat.paste_buffer,
                ) {
                    return ChatNav::Stay;
                }
                chat.paste_buffer = None;
                composer_backspace(chat);
            }
        }
        KeyCode::Delete => {
            if chat.focus == ChatFocus::Composer {
                if consume_large_paste_summary_with_delete(
                    &mut chat.composer,
                    &mut chat.composer_cursor,
                    &mut chat.paste_buffer,
                ) {
                    return ChatNav::Stay;
                }
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

fn handle_double_interrupt_key(
    state: &mut AppState,
    chat: &mut ChatState,
    key_name: &str,
) -> ChatNav {
    if chat.running.is_some() {
        if chat.composer_cleared_by_ctrl_c {
            if let Some(running) = chat.running.take() {
                running.cancellation.cancel();
                running.abort_handle.abort();
            }
            if let Some(started) = chat.run_started_at.take() {
                chat.last_run_elapsed = Some(started.elapsed());
            }
            chat.pending_prompt = None;
            chat.composer_cleared_by_ctrl_c = false;
            push_toast(
                state,
                ToastVariant::Warning,
                "force cancelled",
                Duration::from_secs(2),
            );
            return ChatNav::Stay;
        }
        chat.composer_cleared_by_ctrl_c = true;
        push_toast(
            state,
            ToastVariant::Info,
            format!("press {key_name} again to force cancel"),
            Duration::from_secs(3),
        );
        return ChatNav::Stay;
    }

    if !chat.composer_cleared_by_ctrl_c {
        if !chat.composer.is_empty() {
            composer_clear(chat);
        }
        chat.composer_cleared_by_ctrl_c = true;
        push_toast(
            state,
            ToastVariant::Info,
            format!("press {key_name} again to exit"),
            Duration::from_secs(3),
        );
        return ChatNav::Stay;
    }
    ChatNav::Exit
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

/// Set a specific approval mode and update the shared flag.
///
/// If `chat` is provided and the new mode auto-approves, any pending approval
/// is resolved immediately.
pub(in crate::interactive) fn set_approval_mode(
    state: &mut AppState,
    new_mode: ApprovalMode,
    chat: Option<&mut ChatState>,
) {
    state.approval_mode = new_mode;
    state.mode_flag.store(new_mode as u8, Ordering::Relaxed);
    let toast_msg = format!("{} {} mode", new_mode.icon(), new_mode.label());
    let variant = match new_mode {
        ApprovalMode::Yolo => ToastVariant::Warning,
        ApprovalMode::Plan => ToastVariant::Info,
        ApprovalMode::AcceptEdits => ToastVariant::Warning,
        ApprovalMode::Normal => ToastVariant::Info,
    };
    push_toast(state, variant, toast_msg, Duration::from_secs(3));
    // Auto-resolve pending approval if the new mode would auto-approve.
    if matches!(new_mode, ApprovalMode::Yolo | ApprovalMode::AcceptEdits) {
        if let Some(pending) = state.pending_approval.take() {
            let is_command =
                super::super::approval_is_command_permission(&pending.request.permission);
            let approve = new_mode == ApprovalMode::Yolo || !is_command;
            if approve {
                let _ = pending
                    .reply
                    .send(super::super::ApprovalResponse::AllowOnce);
                state.approval_selection = 0;
                if let Some(chat) = chat {
                    chat.committed_approvals.push(pending.request);
                }
            } else {
                // AcceptEdits but this is a command — put it back for manual approval
                state.pending_approval = Some(super::super::PendingApproval {
                    request: pending.request,
                    reply: pending.reply,
                });
            }
        }
    } else if new_mode == ApprovalMode::Plan {
        // Plan mode auto-denies pending approval
        if let Some(pending) = state.pending_approval.take() {
            let _ = pending.reply.send(super::super::ApprovalResponse::Deny);
            state.approval_selection = 0;
            push_toast(
                state,
                ToastVariant::Info,
                "Plan mode — tool denied",
                Duration::from_secs(2),
            );
        }
    }
}

/// Cycle to the next approval mode.
pub(in crate::interactive) fn cycle_approval_mode(
    state: &mut AppState,
    chat: Option<&mut ChatState>,
) {
    set_approval_mode(state, state.approval_mode.next(), chat);
}
