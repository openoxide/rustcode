use super::super::{
    build_prompt_history, build_transcript_lines, composer_clear, composer_insert_str,
    compute_find_matches, compute_sessions_view, open_command_palette, push_toast, sort_sessions,
    AppState, ChatFocus, ChatNav, ChatState, CommandId, Duration, Modal, ToastVariant,
};
use super::execute::execute_command;
use super::session_ops::{open_session_by_id, refresh_chat_messages};

pub(crate) fn handle_slash_command(
    state: &mut AppState,
    chat: &mut ChatState,
    input: &str,
) -> ChatNav {
    let raw = input.trim();
    let cmd = raw.trim_start_matches('/').trim();
    match cmd {
        // Empty "/" → open command palette so user can discover commands
        "" => {
            open_command_palette(state, Some(chat));
            ChatNav::Stay
        }
        "help" | "?" => {
            state.help_open = true;
            ChatNav::Stay
        }
        // /sessions and /resume with no argument → go to session picker
        "sessions" | "home" | "back" | "resume" => ChatNav::ToSessions,
        "clear" => {
            composer_clear(chat);
            ChatNav::Stay
        }
        "delete" => {
            execute_command(state, CommandId::DeleteSession);
            ChatNav::Stay
        }
        "rename" => {
            execute_command(state, CommandId::RenameSession);
            ChatNav::Stay
        }
        _ if cmd.starts_with("rename ") => {
            let title = cmd.trim_start_matches("rename").trim();
            if title.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "usage: /rename <title>",
                    Duration::from_secs(3),
                );
                return ChatNav::Stay;
            }

            match state
                .backend
                .update_session_title(&chat.session.id, Some(title.to_string()))
            {
                Ok(mut updated) => {
                    // Renaming must never alter the active model label.
                    updated.model = chat.session.model.clone();
                    chat.session = updated.clone();
                    if let Some(idx) = state.sessions.iter().position(|s| s.id == updated.id) {
                        state.sessions[idx] = updated;
                        sort_sessions(&mut state.sessions);
                        state.sessions_view =
                            compute_sessions_view(&state.sessions, &state.sessions_filter);
                    }
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "session renamed",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => push_toast(
                    state,
                    ToastVariant::Error,
                    format!("failed to rename session: {err}"),
                    Duration::from_secs(4),
                ),
            }
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
            ChatNav::Stay
        }
        "thinking" | "reasoning" => {
            chat.show_reasoning = !chat.show_reasoning;
            push_toast(
                state,
                ToastVariant::Info,
                if chat.show_reasoning {
                    "thinking: visible"
                } else {
                    "thinking: hidden"
                },
                Duration::from_secs(2),
            );
            ChatNav::Stay
        }
        "find" | "search" => {
            let query = chat
                .find
                .as_ref()
                .map(|f| f.query.clone())
                .unwrap_or_default();
            let transcript = build_transcript_lines(chat);
            let matches = compute_find_matches(&transcript, &query);
            state.modal = Some(Modal::Search {
                query,
                current: chat.find.as_ref().map_or(0, |f| f.current),
                matches,
            });
            ChatNav::Stay
        }
        "mode" => {
            execute_command(state, CommandId::CycleMode);
            ChatNav::Stay
        }
        "model" => {
            execute_command(state, CommandId::SwitchModel);
            ChatNav::Stay
        }
        "providers" | "auth" | "connect" => {
            execute_command(state, CommandId::ManageProviders);
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
            chat.running = None;
            push_toast(
                state,
                ToastVariant::Success,
                "new session (saved on first message)",
                Duration::from_secs(2),
            );
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
                    chat.output_details = false;
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
        // ── /skill ──────────────────────────────────────────────────────────
        "skill" | "skill list" => {
            let skills = &state.defaults.skills;
            if skills.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Info,
                    "no skills loaded — add .md files to ~/.config/rustcode/skills/",
                    Duration::from_secs(5),
                );
            } else {
                let list: String = skills
                    .iter()
                    .filter(|s| s.metadata.enabled)
                    .map(|s| format!("  {} — {}", s.metadata.name, s.metadata.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                push_toast(
                    state,
                    ToastVariant::Info,
                    format!("skills:\n{list}"),
                    Duration::from_secs(6),
                );
            }
            ChatNav::Stay
        }
        _ if cmd.starts_with("skill ") => {
            let skill_name = cmd.trim_start_matches("skill ").trim();
            match state
                .defaults
                .skills
                .iter()
                .find(|s| s.metadata.name == skill_name && s.metadata.enabled)
            {
                Some(skill) => {
                    let prefix = format!("[skill: {}]\n", skill.metadata.name);
                    let injection = format!("{}{}\n\n", prefix, skill.content.trim());
                    composer_insert_str(chat, &injection);
                    push_toast(
                        state,
                        ToastVariant::Success,
                        format!("injected skill: {}", skill.metadata.name),
                        Duration::from_secs(3),
                    );
                }
                None => {
                    push_toast(
                        state,
                        ToastVariant::Warning,
                        format!("skill not found: {skill_name}"),
                        Duration::from_secs(3),
                    );
                }
            }
            ChatNav::Stay
        }
        // ── /memory ─────────────────────────────────────────────────────────
        "memory" => {
            open_memory_viewer(state);
            ChatNav::Stay
        }
        "memory on" => {
            match rustcode_memories::MemoryStorage::new() {
                Some(storage) => match storage.set_enabled(true) {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Success,
                        "memory: enabled",
                        Duration::from_secs(3),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed: {err}"),
                        Duration::from_secs(4),
                    ),
                },
                None => push_toast(
                    state,
                    ToastVariant::Warning,
                    "memory storage unavailable",
                    Duration::from_secs(3),
                ),
            }
            ChatNav::Stay
        }
        "memory off" => {
            match rustcode_memories::MemoryStorage::new() {
                Some(storage) => match storage.set_enabled(false) {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Warning,
                        "memory: disabled",
                        Duration::from_secs(3),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed: {err}"),
                        Duration::from_secs(4),
                    ),
                },
                None => push_toast(
                    state,
                    ToastVariant::Warning,
                    "memory storage unavailable",
                    Duration::from_secs(3),
                ),
            }
            ChatNav::Stay
        }
        "memory clear" => {
            state.modal = Some(Modal::MemoryClearConfirm);
            ChatNav::Stay
        }
        // /sessions <id> or /resume <id> → open session by ID
        _ if cmd.starts_with("sessions ") || cmd.starts_with("resume ") => {
            let session_id = cmd.split_once(' ').map_or("", |x| x.1).trim();
            if session_id.is_empty() {
                return ChatNav::ToSessions;
            }
            open_session_by_id(state, session_id);
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

/// Open the full memory viewer modal.
pub(crate) fn open_memory_viewer(state: &mut AppState) {
    let (content, raw_count, updated_at, enabled) = match rustcode_memories::MemoryStorage::new() {
        Some(storage) => {
            let enabled = !storage.is_disabled();
            let raw_count = storage.raw_count();
            match storage.load_summary() {
                Some(summary) => (summary.content, raw_count, summary.updated_at, enabled),
                None => (
                    "No memory summary yet.\n\nComplete a session to start building memories."
                        .to_string(),
                    raw_count,
                    0,
                    enabled,
                ),
            }
        }
        None => (
            "Memory storage unavailable (HOME not set).".to_string(),
            0,
            0,
            false,
        ),
    };
    let total_lines = content.lines().count();
    state.modal = Some(Modal::MemoryViewer {
        content,
        raw_count,
        updated_at,
        enabled,
        scroll: 0,
        total_lines,
    });
}
