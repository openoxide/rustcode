use super::{
    approval_is_command_permission, approval_options_count, build_provider_entries,
    composer_backspace, composer_clear, composer_delete, composer_insert_str,
    composer_kill_line_backward, composer_kill_line_forward, composer_move_down, composer_move_end,
    composer_move_home, composer_move_left, composer_move_right, composer_move_up,
    composer_word_left, composer_word_right, compute_palette_view, compute_sessions_view,
    execute_command, filter_files, filter_models, filter_provider_entries, find_next, find_prev,
    handle_slash_command, history_next, history_prev, maybe_execute_palette_query,
    open_command_palette, provider_connect_methods, provider_display_name, provider_env_hint,
    push_toast, refresh_chat_messages, set_find, sort_sessions, submit_prompt,
    transcript_area_height, AppState, ApprovalResponse, Arc, ChatFocus, ChatNav, ChatState,
    CommandId, ConnectMethod, Duration, KeyCode, KeyEvent, KeyModifiers, Modal,
    ProviderManagerStep, ProviderOAuthDone, ProviderOAuthStarted, Screen, ToastVariant,
    SLASH_COMMANDS,
};

mod input_chat;
mod input_memory;
mod input_modals;
mod input_pm;
mod input_provider;
mod input_sessions;

use self::input_chat::handle_chat_key;
pub(in crate::interactive) use self::input_chat::set_approval_mode;
use self::input_modals::handle_modal_key;
use self::input_sessions::handle_sessions_key;

pub(super) fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    // Modals take priority over everything — the user must dismiss the modal
    // before interacting with approval prompts or the underlying screen.
    if state.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => state.help_open = false,
            _ => {}
        }
        return false;
    }

    if state.modal.is_some() && !matches!(&state.modal, Some(Modal::SlashHelp { .. })) {
        handle_modal_key(state, key);
        return false;
    }

    // Pending tool approval — blocks all other input until resolved.
    if let Some((is_command, options_count)) = state.pending_approval.as_ref().map(|pending| {
        let request = &pending.request;
        (
            approval_is_command_permission(&request.permission),
            approval_options_count(request),
        )
    }) {
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
                if let Some(pending) = state.pending_approval.take() {
                    let response = match state.approval_selection {
                        0 => ApprovalResponse::AllowOnce,
                        1 if is_command => ApprovalResponse::AllowAllCommands,
                        1 => ApprovalResponse::AllowAllToolsAutopilot,
                        2 if is_command => ApprovalResponse::AllowAllToolsAutopilot,
                        _ => ApprovalResponse::Deny,
                    };
                    let label = match response {
                        ApprovalResponse::AllowOnce => {
                            format!("approved once [{}]", pending.request.tool)
                        }
                        ApprovalResponse::AllowAllCommands => "allowed all edits".to_string(),
                        ApprovalResponse::AllowAllToolsAutopilot => {
                            "approved all tools (auto-pilot mode)".to_string()
                        }
                        ApprovalResponse::Deny => format!("denied [{}]", pending.request.tool),
                    };
                    push_toast(state, ToastVariant::Info, label, Duration::from_secs(2));
                    let _ = pending.reply.send(response);
                    state.approval_selection = 0;
                    if matches!(response, ApprovalResponse::AllowOnce) {
                        if let Screen::Chat(chat) = &mut state.screen {
                            chat.committed_approvals.push(super::CachedApproval {
                                request: pending.request,
                                lines: Vec::new(),
                                width: 0,
                            });
                            chat.transcript_dirty.set(true);
                        }
                    }
                    // Switch TUI approval mode to match the selected policy.
                    if matches!(response, ApprovalResponse::AllowAllToolsAutopilot) {
                        let screen = std::mem::replace(&mut state.screen, Screen::Sessions);
                        if let Screen::Chat(mut chat) = screen {
                            set_approval_mode(state, super::ApprovalMode::Yolo, Some(&mut chat));
                            state.screen = Screen::Chat(chat);
                        } else {
                            state.screen = screen;
                            set_approval_mode(state, super::ApprovalMode::Yolo, None);
                        }
                    } else if matches!(response, ApprovalResponse::AllowAllCommands) {
                        let screen = std::mem::replace(&mut state.screen, Screen::Sessions);
                        if let Screen::Chat(mut chat) = screen {
                            set_approval_mode(
                                state,
                                super::ApprovalMode::AcceptEdits,
                                Some(&mut chat),
                            );
                            state.screen = Screen::Chat(chat);
                        } else {
                            state.screen = screen;
                            set_approval_mode(state, super::ApprovalMode::AcceptEdits, None);
                        }
                    }
                }
            }
            KeyCode::Esc => {
                if let Some(pending) = state.pending_approval.take() {
                    push_toast(
                        state,
                        ToastVariant::Info,
                        format!("denied [{}]", pending.request.tool),
                        Duration::from_secs(2),
                    );
                    let _ = pending.reply.send(ApprovalResponse::Deny);
                    state.approval_selection = 0;
                }
            }
            // Shift+Tab: cycle approval mode (may auto-resolve this pending approval)
            KeyCode::BackTab => {
                // Extract chat mutably for cycle_approval_mode
                let screen = std::mem::replace(&mut state.screen, Screen::Sessions);
                if let Screen::Chat(mut chat) = screen {
                    input_chat::cycle_approval_mode(state, Some(&mut chat));
                    state.screen = Screen::Chat(chat);
                } else {
                    state.screen = screen;
                    input_chat::cycle_approval_mode(state, None);
                }
            }
            _ => {}
        }
        return false;
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
            let stem = cmd.trim_start_matches('/');
            let stem = stem.split_whitespace().next().unwrap_or(stem);
            stem.to_ascii_lowercase().contains(&needle)
        })
        .copied()
        .collect()
}

/// Sync the [`Modal::SlashHelp`] popup with the current composer content.
fn sync_slash_help(state: &mut AppState, chat: &ChatState) {
    if chat.composer.starts_with('/') && !chat.composer.contains('\n') {
        let query = chat.composer.trim_start_matches('/').to_string();
        match &mut state.modal {
            Some(Modal::SlashHelp { query: q, .. }) => *q = query,
            _ => state.modal = Some(Modal::SlashHelp { query, selected: 0 }),
        }
    } else if matches!(state.modal, Some(Modal::SlashHelp { .. })) {
        state.modal = None;
    }
}
