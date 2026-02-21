use super::{
    build_prompt_history, build_provider_entries, build_transcript_lines, composer_clear,
    composer_insert_str, compute_find_matches, compute_sessions_view, filter_files,
    filter_provider_entries, open_command_palette, push_toast, scan_workspace_files, sort_sessions,
    AppState, ChatFocus, ChatNav, ChatState, CommandId, CreateSessionOptions, Duration, Modal,
    ProviderManagerStep, Screen, ToastVariant,
};

/// All slash commands available in the composer, with short descriptions.
///
/// Shown in the [`Modal::SlashHelp`] autocomplete popup.
pub(super) const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/clear", "Clear composer input"),
    ("/delete", "Delete current session"),
    ("/find <query>", "Search transcript"),
    ("/fork", "Fork current session  (Ctrl+F)"),
    ("/help", "Show keybindings and tips"),
    ("/memory", "Show memory summary"),
    ("/model", "Switch LLM model  (Ctrl+M)"),
    ("/new", "Create a new session  (Ctrl+N)"),
    ("/providers", "Connect/disconnect providers  (Ctrl+A)"),
    ("/refresh", "Reload transcript  (Ctrl+R)"),
    ("/rename <title>", "Rename current session"),
    (
        "/resume <id>",
        "Resume a session by ID (empty = session list)",
    ),
    ("/sessions", "Go to sessions screen  (Ctrl+Q)"),
    ("/skill", "List or inject a skill  (Ctrl+S)"),
    ("/thinking", "Toggle thinking/reasoning visibility  (Ctrl+Y)"),
    ("/tools", "Toggle tool call details  (Ctrl+D)"),
];

/// Filter `entries` (each a `"provider/model"` string) by case-insensitive substring match.
pub(super) fn filter_models(entries: &[String], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    let needle = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.to_ascii_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

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
                        live_reasoning: String::new(),
                        show_reasoning: false,
                        composer: String::new(),
                        composer_cursor: 0,
                        paste_buffer: None,
                        prompt_history,
                        history_cursor: None,
                        history_draft: String::new(),
                        focus: ChatFocus::Composer,
                        activity: Vec::new(),
                        activity_selected: 0,
                        details_open: false,
                        activity_hidden: false,
                        tool_details: true,
                        find: None,
                        running: None,
                        pending_prompt: None,
                        composer_cleared_by_ctrl_c: false,
                        last_typing_time: None,
                        total_input_tokens: 0,
                        total_output_tokens: 0,
                        last_total_tokens: 0,
                        context_limit: 0,
                        cost_usd: 0.0,
                        last_max_scroll: std::cell::Cell::new(0),
                        run_started_at: None,
                        last_run_elapsed: None,
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
                    chat.tool_details = true;
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
        CommandId::ToggleReasoning => {
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
            let Screen::Chat(chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
                push_toast(
                    state,
                    ToastVariant::Warning,
                    "open a session first",
                    Duration::from_secs(3),
                );
                return;
            };
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
        CommandId::ToggleActivity => {
            if let Screen::Chat(chat) = &mut state.screen {
                chat.activity_hidden = !chat.activity_hidden;
                // If activity was focused and we just hid it, move focus to Composer
                if chat.activity_hidden && chat.focus == ChatFocus::Activity {
                    chat.focus = ChatFocus::Composer;
                }
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
        CommandId::SwitchModel => {
            let entries = build_model_entries(&state.defaults.model);
            let view = filter_models(&entries, "");
            state.modal = Some(Modal::ModelSelect {
                current_model: state.defaults.model.clone(),
                entries,
                query: String::new(),
                view,
                selected: 0,
            });
        }
        CommandId::ManageProviders => {
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
    }
}

/// Build the ordered list of `"provider/model"` entries for the model picker.
///
/// Only shows models from providers that have credentials configured (env vars
/// or auth store). Falls back to all built-in models if nothing is connected.
/// The current model's provider is surfaced first; priority providers follow.
fn build_model_entries(current_model: &str) -> Vec<String> {
    let current_provider = current_model
        .split_once('/')
        .map(|(p, _)| p)
        .unwrap_or("")
        .to_ascii_lowercase();

    const PRIORITY: &[&str] = &[
        "anthropic",
        "openai",
        "google",
        "groq",
        "xai",
        "mistral",
        "deepseek",
        "deepinfra",
        "cerebras",
        "togetherai",
        "perplexity",
        "ollama",
        "openrouter",
        "azure",
    ];

    // Determine which providers have credentials configured.
    let connected: std::collections::HashSet<String> =
        rustcode_llm::connected_provider_ids().into_iter().collect();

    // Get every known model entry (built-in + shared registry + flat external).
    let all = rustcode_llm::all_model_entries();

    // Filter to only connected providers; if none configured, show everything.
    let filtered: Vec<String> = if connected.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|entry| {
                let provider = entry.split('/').next().unwrap_or("");
                connected.contains(provider)
            })
            .collect()
    };

    // Sort: current provider first, then priority order, then alpha.
    let priority_rank = |provider: &str| -> usize {
        if provider == current_provider {
            return 0;
        }
        PRIORITY
            .iter()
            .position(|&p| p == provider)
            .map(|i| i + 1)
            .unwrap_or(usize::MAX)
    };

    let mut entries = filtered;
    entries.sort_by(|a, b| {
        let pa = a.split('/').next().unwrap_or("");
        let pb = b.split('/').next().unwrap_or("");
        priority_rank(pa)
            .cmp(&priority_rank(pb))
            .then_with(|| pa.cmp(pb))
            .then_with(|| a.cmp(b))
    });

    entries
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
                Ok(updated) => {
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
                    chat.tool_details = true;
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
                    chat.tool_details = true;
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
        // /sessions <id> or /resume <id> → open session by ID
        _ if cmd.starts_with("sessions ") || cmd.starts_with("resume ") => {
            let session_id = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim();
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

/// Open a session by its ID, loading messages and switching to the chat screen.
///
/// Used by `/sessions <id>` and `/resume <id>` slash commands and the palette
/// `session <id>` verb. Token usage and cost are read from the persisted
/// session metadata (saved on each successful run completion).
pub(super) fn open_session_by_id(state: &mut AppState, session_id: &str) {
    let query = session_id.trim();
    if query.is_empty() {
        push_toast(
            state,
            ToastVariant::Warning,
            "usage: /sessions <id|query>",
            Duration::from_secs(3),
        );
        return;
    }

    if let Ok(session) = state.backend.get_session(query) {
        open_session(state, session);
        return;
    }

    let mut sessions = match state.backend.list_sessions() {
        Ok(items) => items,
        Err(err) => {
            push_toast(
                state,
                ToastVariant::Error,
                format!("failed to list sessions: {err}"),
                Duration::from_secs(4),
            );
            return;
        }
    };
    sort_sessions(&mut sessions);
    let view = compute_sessions_view(&sessions, query);
    match view.len() {
        0 => push_toast(
            state,
            ToastVariant::Error,
            format!("session not found: {query}"),
            Duration::from_secs(4),
        ),
        1 => {
            let idx = view[0];
            if let Some(session) = sessions.get(idx).cloned() {
                open_session(state, session);
            }
        }
        count => {
            state.sessions = sessions;
            state.sessions_filter = query.to_string();
            state.sessions_filter_active = true;
            state.sessions_view = view;
            state.selected = 0;
            state.screen = Screen::Sessions;
            push_toast(
                state,
                ToastVariant::Info,
                format!("{count} sessions matched \"{query}\""),
                Duration::from_secs(3),
            );
        }
    }
}

fn open_session(state: &mut AppState, session: rustcode_core::SessionInfo) {
    let session_id = session.id.clone();
    let messages = state
        .backend
        .load_messages(&session_id)
        .unwrap_or_else(|err| {
            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
            Vec::new()
        });
    let prompt_history = build_prompt_history(&messages);
    state.screen = Screen::Chat(ChatState {
        total_input_tokens: session.total_input_tokens,
        total_output_tokens: session.total_output_tokens,
        last_total_tokens: session.total_input_tokens + session.total_output_tokens,
        cost_usd: session.cost_usd,
        session,
        messages,
        scroll: 0,
        live_assistant: String::new(),
        live_reasoning: String::new(),
        show_reasoning: false,
        composer: String::new(),
        composer_cursor: 0,
        paste_buffer: None,
        prompt_history,
        history_cursor: None,
        history_draft: String::new(),
        focus: ChatFocus::Composer,
        activity: Vec::new(),
        activity_selected: 0,
        details_open: false,
        activity_hidden: false,
        tool_details: true,
        find: None,
        running: None,
        pending_prompt: None,
        composer_cleared_by_ctrl_c: false,
        last_typing_time: None,
        context_limit: 0,
        last_max_scroll: std::cell::Cell::new(0),
        run_started_at: None,
        last_run_elapsed: None,
    });
    if let Some(idx) = state.sessions.iter().position(|s| s.id == session_id) {
        state.selected = idx;
    }
}
