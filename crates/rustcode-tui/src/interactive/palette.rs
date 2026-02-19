use super::{
    build_prompt_history, build_transcript_lines, compute_find_matches, execute_command,
    push_toast, set_find, transcript_area_height, AppState, ChatFocus, ChatState, CommandId,
    CommandItem, Duration, InteractiveSubmitMode, Modal, Screen, ToastVariant,
};

pub(super) fn open_command_palette(state: &mut AppState) {
    let items = build_command_items(state);
    let view = compute_palette_view(&items, "");
    state.modal = Some(Modal::CommandPalette {
        query: String::new(),
        selected: 0,
        items,
        view,
    });
}

pub(super) fn build_command_items(state: &AppState) -> Vec<CommandItem> {
    let mut items = Vec::new();

    let (in_chat, chat_running, attach_mode) = match &state.screen {
        Screen::Chat(chat) => (
            true,
            chat.running.is_some(),
            state.submit_mode == InteractiveSubmitMode::Run,
        ),
        Screen::Sessions => (
            false,
            false,
            state.submit_mode == InteractiveSubmitMode::Run,
        ),
    };

    let can_fork = !attach_mode;
    let can_rename_delete = !attach_mode;

    items.push(CommandItem {
        id: CommandId::Help,
        title: "Help".to_string(),
        detail: "Show keybinds and tips".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::Sessions,
        title: "Sessions".to_string(),
        detail: "Go to session picker".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::NewSession,
        title: "New session".to_string(),
        detail: "Create a new session".to_string(),
        enabled: !chat_running,
        disabled_reason: chat_running.then_some("Cannot create session while running".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::ForkSession,
        title: "Fork session".to_string(),
        detail: "Fork current session".to_string(),
        enabled: in_chat && !chat_running && can_fork,
        disabled_reason: if !in_chat {
            Some("Open a session first".to_string())
        } else if chat_running {
            Some("Cannot fork session while running".to_string())
        } else if !can_fork {
            Some("Not supported in attach mode".to_string())
        } else {
            None
        },
    });

    items.push(CommandItem {
        id: CommandId::RenameSession,
        title: "Rename session".to_string(),
        detail: "Edit the session title".to_string(),
        enabled: can_rename_delete,
        disabled_reason: (!can_rename_delete).then_some("Not supported in attach mode".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::DeleteSession,
        title: "Delete session".to_string(),
        detail: "Delete selected session".to_string(),
        enabled: can_rename_delete,
        disabled_reason: (!can_rename_delete).then_some("Not supported in attach mode".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::Refresh,
        title: "Refresh".to_string(),
        detail: "Reload sessions or transcript".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::ToggleTools,
        title: "Toggle tools".to_string(),
        detail: "Toggle tool transcript details".to_string(),
        enabled: in_chat,
        disabled_reason: (!in_chat).then_some("Open a session first".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::Search,
        title: "Search transcript".to_string(),
        detail: "Find in transcript".to_string(),
        enabled: in_chat,
        disabled_reason: (!in_chat).then_some("Open a session first".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::FileSearch,
        title: "File search".to_string(),
        detail: "Browse workspace files (Ctrl+T)".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::FocusComposer,
        title: "Focus composer".to_string(),
        detail: "Move focus to prompt input".to_string(),
        enabled: in_chat,
        disabled_reason: (!in_chat).then_some("Open a session first".to_string()),
    });
    items.push(CommandItem {
        id: CommandId::FocusTranscript,
        title: "Focus transcript".to_string(),
        detail: "Move focus to transcript".to_string(),
        enabled: in_chat,
        disabled_reason: (!in_chat).then_some("Open a session first".to_string()),
    });
    items.push(CommandItem {
        id: CommandId::FocusActivity,
        title: "Focus activity".to_string(),
        detail: "Move focus to activity panel".to_string(),
        enabled: in_chat,
        disabled_reason: (!in_chat).then_some("Open a session first".to_string()),
    });

    items.push(CommandItem {
        id: CommandId::CancelRun,
        title: "Cancel run".to_string(),
        detail: "Cancel the running command".to_string(),
        enabled: in_chat && chat_running,
        disabled_reason: if !in_chat {
            Some("Open a session first".to_string())
        } else if !chat_running {
            Some("Nothing is running".to_string())
        } else {
            None
        },
    });

    items.push(CommandItem {
        id: CommandId::ToggleSkills,
        title: "Skills".to_string(),
        detail: "Toggle enabled/disabled skills (Ctrl+S)".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::Feedback,
        title: "Feedback".to_string(),
        detail: "Rate this session with thumbs up/down (Ctrl+B)".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::SwitchModel,
        title: "Switch model".to_string(),
        detail: "Pick a different LLM model for this session (Ctrl+M)".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::ManageProviders,
        title: "Providers".to_string(),
        detail: "Connect/disconnect LLM providers (Ctrl+A)".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items.push(CommandItem {
        id: CommandId::Quit,
        title: "Quit".to_string(),
        detail: "Exit the app".to_string(),
        enabled: true,
        disabled_reason: None,
    });

    items
}

pub(super) fn compute_palette_view(items: &[CommandItem], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..items.len()).collect();
    }
    let needle = query.trim().to_ascii_lowercase();
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let hay = format!("{}\t{}", item.title, item.detail).to_ascii_lowercase();
        if hay.contains(&needle) {
            out.push(idx);
        }
    }
    out
}

pub(super) fn maybe_execute_palette_query(state: &mut AppState, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }

    let (verb, rest) = trimmed
        .split_once(' ')
        .map_or((trimmed, ""), |(a, b)| (a, b.trim()));
    let verb = verb.to_ascii_lowercase();

    match verb.as_str() {
        "help" => {
            state.help_open = true;
            true
        }
        "sessions" | "home" => {
            state.screen = Screen::Sessions;
            true
        }
        "new" => {
            execute_command(state, CommandId::NewSession);
            true
        }
        "fork" => {
            execute_command(state, CommandId::ForkSession);
            true
        }
        "refresh" | "reload" => {
            execute_command(state, CommandId::Refresh);
            true
        }
        "tools" => {
            execute_command(state, CommandId::ToggleTools);
            true
        }
        "model" | "models" => {
            execute_command(state, CommandId::SwitchModel);
            true
        }
        "providers" | "auth" | "connect" => {
            execute_command(state, CommandId::ManageProviders);
            true
        }
        "search" | "find" => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return true;
            };
            chat.focus = ChatFocus::Transcript;
            let viewport_h = transcript_area_height(state);
            set_find(state, &mut chat, rest.to_string(), true, viewport_h);
            let query = chat
                .find
                .as_ref()
                .map(|f| f.query.clone())
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
            true
        }
        "focus" => {
            match rest.to_ascii_lowercase().as_str() {
                "composer" | "prompt" => execute_command(state, CommandId::FocusComposer),
                "transcript" | "chat" => execute_command(state, CommandId::FocusTranscript),
                "activity" => execute_command(state, CommandId::FocusActivity),
                _ => push_toast(
                    state,
                    ToastVariant::Warning,
                    "focus: composer|transcript|activity",
                    Duration::from_secs(3),
                ),
            }
            true
        }
        "open" | "session" => {
            let session_id = rest;
            if session_id.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "usage: session <id>",
                    Duration::from_secs(3),
                );
                return true;
            }
            match state.backend.get_session(session_id) {
                Ok(session) => {
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
                        pending_prompt: None,
                        composer_cleared_by_ctrl_c: false,
                        last_typing_time: None,
                    });
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "opened session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to open session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
            true
        }
        "rename" => {
            if rest.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "usage: rename <title>",
                    Duration::from_secs(3),
                );
                return true;
            }
            let (target_id, title) = if let Some((maybe_id, title)) = rest.split_once(' ') {
                if maybe_id.starts_with('s') && !title.trim().is_empty() {
                    (Some(maybe_id.to_string()), title.trim().to_string())
                } else {
                    (None, rest.to_string())
                }
            } else {
                (None, rest.to_string())
            };

            let session_id = if let Some(id) = target_id {
                id
            } else {
                match &state.screen {
                    Screen::Chat(chat) => chat.session.id.clone(),
                    Screen::Sessions => state
                        .sessions_view
                        .get(state.selected)
                        .and_then(|idx| state.sessions.get(*idx))
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                }
            };
            if session_id.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "no session selected",
                    Duration::from_secs(3),
                );
                return true;
            }
            state.modal = Some(Modal::Rename {
                session_id,
                cursor: title.chars().count(),
                input: title,
            });
            true
        }
        "delete" => {
            let session_id = if rest.is_empty() {
                match &state.screen {
                    Screen::Chat(chat) => chat.session.id.clone(),
                    Screen::Sessions => state
                        .sessions_view
                        .get(state.selected)
                        .and_then(|idx| state.sessions.get(*idx))
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                }
            } else {
                rest.to_string()
            };
            if session_id.is_empty() {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "usage: delete <session_id>",
                    Duration::from_secs(3),
                );
                return true;
            }
            let title = state
                .sessions
                .iter()
                .find(|s| s.id == session_id)
                .and_then(|s| s.title.clone())
                .unwrap_or_else(|| session_id.clone());
            state.modal = Some(Modal::DeleteConfirm { session_id, title });
            true
        }
        _ => false,
    }
}
