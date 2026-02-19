use super::{
    build_prompt_history, build_transcript_lines, composer_clear, composer_insert_str,
    compute_find_matches, compute_sessions_view, filter_files, open_command_palette, push_toast,
    scan_workspace_files, sort_sessions, AppState, ChatFocus, ChatNav, ChatState, CommandId,
    CreateSessionOptions, Duration, Modal, Screen, ToastVariant,
};

pub(super) fn execute_command(state: &mut AppState, id: CommandId) {
    match id {
        CommandId::Help => state.help_open = true,
        CommandId::Sessions => state.screen = Screen::Sessions,
        CommandId::NewSession => {
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to resolve cwd: {err}"),
                        Duration::from_secs(4),
                    );
                    return;
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
                        push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                        Vec::new()
                    });
                    sort_sessions(&mut state.sessions);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = state
                        .sessions
                        .iter()
                        .position(|s| s.id == session.id)
                        .unwrap_or(0);

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
                        "created session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to create session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
        }
        CommandId::ForkSession => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
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
                    composer_clear(&mut chat);
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
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "forked session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => push_toast(state, ToastVariant::Error, err, Duration::from_secs(4)),
            }
            state.screen = Screen::Chat(chat);
        }
        CommandId::RenameSession => match &state.screen {
            Screen::Sessions => {
                let Some(session) = state
                    .sessions_view
                    .get(state.selected)
                    .and_then(|idx| state.sessions.get(*idx))
                    .cloned()
                else {
                    return;
                };
                let input = session.title.clone().unwrap_or_default();
                state.modal = Some(Modal::Rename {
                    session_id: session.id,
                    cursor: input.chars().count(),
                    input,
                });
            }
            Screen::Chat(chat) => {
                let input = chat.session.title.clone().unwrap_or_default();
                state.modal = Some(Modal::Rename {
                    session_id: chat.session.id.clone(),
                    cursor: input.chars().count(),
                    input,
                });
            }
        },
        CommandId::DeleteSession => {
            let (session_id, title) = match &state.screen {
                Screen::Sessions => {
                    let Some(session) = state
                        .sessions_view
                        .get(state.selected)
                        .and_then(|idx| state.sessions.get(*idx))
                        .cloned()
                    else {
                        return;
                    };
                    (
                        session.id.clone(),
                        session.title.clone().unwrap_or_else(|| session.id.clone()),
                    )
                }
                Screen::Chat(chat) => (
                    chat.session.id.clone(),
                    chat.session
                        .title
                        .clone()
                        .unwrap_or_else(|| chat.session.id.clone()),
                ),
            };
            state.modal = Some(Modal::DeleteConfirm { session_id, title });
        }
        CommandId::Refresh => match &state.screen {
            Screen::Sessions => {
                state.sessions = state.backend.list_sessions().unwrap_or_else(|err| {
                    push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                    Vec::new()
                });
                sort_sessions(&mut state.sessions);
                state.sessions_view =
                    compute_sessions_view(&state.sessions, &state.sessions_filter);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "refreshed sessions",
                    Duration::from_secs(2),
                );
            }
            Screen::Chat(_) => {
                let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
                else {
                    return;
                };
                refresh_chat_messages(state, &mut chat);
                state.screen = Screen::Chat(chat);
                push_toast(
                    state,
                    ToastVariant::Info,
                    "refreshed transcript",
                    Duration::from_secs(2),
                );
            }
        },
        CommandId::ToggleTools => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
            chat.tool_details = !chat.tool_details;
            push_toast(
                state,
                ToastVariant::Info,
                if chat.tool_details {
                    "tools: details"
                } else {
                    "tools: summary"
                },
                Duration::from_secs(2),
            );
            state.screen = Screen::Chat(chat);
        }
        CommandId::FileSearch => {
            let entries = scan_workspace_files(&state.defaults.workspace_root.clone());
            let view = filter_files(&entries, "");
            state.modal = Some(Modal::FileSearch {
                query: String::new(),
                entries,
                view,
                selected: 0,
            });
        }
        CommandId::Search => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
            chat.focus = ChatFocus::Transcript;
            let query = chat
                .find
                .as_ref()
                .map(|find| find.query.clone())
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
        }
        CommandId::FocusComposer => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Composer;
            }
        }
        CommandId::FocusTranscript => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Transcript;
            }
        }
        CommandId::FocusActivity => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.focus = ChatFocus::Activity;
            }
        }
        CommandId::CancelRun => {
            if let Screen::Chat(chat) = &mut state.screen {
                if let Some(running) = &chat.running {
                    running.cancellation.cancel();
                    push_toast(
                        state,
                        ToastVariant::Warning,
                        "cancel requested",
                        Duration::from_secs(2),
                    );
                }
            }
        }
        CommandId::Quit => push_toast(
            state,
            ToastVariant::Info,
            "press q to quit",
            Duration::from_secs(2),
        ),
        CommandId::ToggleSkills => {
            let skills = state
                .defaults
                .skills
                .iter()
                .map(|s| {
                    (
                        s.name().to_string(),
                        s.description().to_string(),
                        s.metadata.enabled,
                    )
                })
                .collect();
            state.modal = Some(Modal::SkillToggle {
                skills,
                selected: 0,
            });
        }
        CommandId::Feedback => {
            state.modal = Some(Modal::Feedback {
                rating: None,
                comment: String::new(),
                comment_active: false,
            });
        }
    }
}

pub(super) fn handle_slash_command(
    state: &mut AppState,
    chat: &mut ChatState,
    input: &str,
) -> ChatNav {
    let raw = input.trim();
    let cmd = raw.trim_start_matches('/').trim();
    match cmd {
        // Empty "/" → open command palette so user can discover commands
        "" => {
            open_command_palette(state);
            ChatNav::Stay
        }
        "help" | "?" => {
            state.help_open = true;
            ChatNav::Stay
        }
        "sessions" | "home" | "back" => ChatNav::ToSessions,
        "clear" => {
            composer_clear(chat);
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
            chat.tool_details = !chat.tool_details;
            push_toast(
                state,
                ToastVariant::Info,
                if chat.tool_details {
                    "tools: details"
                } else {
                    "tools: summary"
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
            chat.focus = ChatFocus::Transcript;
            ChatNav::Stay
        }
        "model" => {
            let model = state.defaults.model.clone();
            push_toast(
                state,
                ToastVariant::Info,
                format!("model: {model}"),
                Duration::from_secs(4),
            );
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
                    chat.running = None;
                    push_toast(
                        state,
                        ToastVariant::Success,
                        "created session",
                        Duration::from_secs(2),
                    );
                }
                Err(err) => {
                    push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to create session: {err}"),
                        Duration::from_secs(4),
                    );
                }
            }
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
            match rustcode_memories::MemoryStorage::new().and_then(|s| s.load_summary()) {
                Some(summary) => {
                    let excerpt: String = summary.content.chars().take(200).collect();
                    let suffix = if summary.content.len() > 200 {
                        "…"
                    } else {
                        ""
                    };
                    push_toast(
                        state,
                        ToastVariant::Info,
                        format!("memory summary:\n{excerpt}{suffix}"),
                        Duration::from_secs(7),
                    );
                }
                None => {
                    push_toast(
                        state,
                        ToastVariant::Info,
                        "no memory summary yet — complete a session to build memories",
                        Duration::from_secs(5),
                    );
                }
            }
            ChatNav::Stay
        }
        "memory clear" => {
            match rustcode_memories::MemoryStorage::new() {
                Some(storage) => match storage.clear_raw() {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Success,
                        "raw memories cleared",
                        Duration::from_secs(3),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to clear memories: {err}"),
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

pub(super) fn refresh_chat_messages(state: &mut AppState, chat: &mut ChatState) {
    match state.backend.load_messages(&chat.session.id) {
        Ok(messages) => chat.messages = messages,
        Err(err) => {
            state.status = Some(format!("failed to load transcript: {err}"));
            push_toast(
                state,
                ToastVariant::Error,
                format!("failed to load transcript: {err}"),
                Duration::from_secs(4),
            );
        }
    }
}
