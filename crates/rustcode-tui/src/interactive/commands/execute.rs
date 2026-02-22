use std::collections::VecDeque;

use super::super::{
    build_provider_entries, build_transcript_lines, composer_clear, compute_find_matches,
    compute_sessions_view, filter_files, filter_provider_entries, push_toast, scan_workspace_files,
    sort_sessions, AppState, ChatFocus, ChatState, CommandId, Duration, Modal, ProviderManagerStep,
    Screen, ToastVariant,
};
use super::filter_models;
use super::session_ops::refresh_chat_messages;
use super::slash::open_memory_viewer;

const MODEL_PICKER_PROVIDER_PRIORITY: &[&str] = &[
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

pub(crate) fn execute_command(state: &mut AppState, id: CommandId) {
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
            let session = crate::new_draft_session(
                None,
                cwd,
                state.defaults.workspace_root.clone(),
                state.defaults.model.clone(),
            );
            state.screen = Screen::Chat(ChatState {
                session,
                messages: Vec::new(),
                scroll: 0,
                live_assistant: String::new(),
                live_reasoning: String::new(),
                show_reasoning: false,
                composer: String::new(),
                composer_cursor: 0,
                paste_buffer: None,
                focus: ChatFocus::Composer,
                activity: VecDeque::new(),
                activity_selected: 0,
                details_open: false,
                activity_hidden: true,
                tool_details: false,
                output_details: false,
                find: None,
                running: None,
                pending_prompt: None,
                committed_approvals: Vec::new(),
                composer_cleared_by_ctrl_c: false,
                last_typing_time: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                last_total_tokens: 0,
                context_limit: 0,
                cost_usd: 0.0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                last_max_scroll: std::cell::Cell::new(0),
                last_transcript_wrapped_count: std::cell::Cell::new(0),
                run_started_at: None,
                last_run_elapsed: None,
                plan_title: None,
                plan_steps: Vec::new(),
                todos: Vec::new(),
                cached_transcript: std::cell::RefCell::new(Vec::new()),
                transcript_dirty: std::cell::Cell::new(true),
                last_transcript_width: std::cell::Cell::new(0),
            });
            push_toast(
                state,
                ToastVariant::Success,
                "new session (saved on first message)",
                Duration::from_secs(2),
            );
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
                    state.history_cursor = None;
                    state.history_draft.clear();
                    chat.focus = ChatFocus::Composer;
                    chat.activity.clear();
                    chat.activity_selected = 0;
                    chat.details_open = false;
                    chat.tool_details = false;
                    chat.output_details = false;
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
                if let Some(running) = chat.running.take() {
                    running.cancellation.cancel();
                    running.abort_handle.abort();
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
        CommandId::ViewMemory => {
            open_memory_viewer(state);
        }
        CommandId::CycleMode => {
            state.modal = Some(Modal::ModeSelect {
                selected: state.approval_mode as usize,
            });
        }
        CommandId::Compact => {
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
            super::super::submit_compact(state, &mut chat, None);
            state.screen = Screen::Chat(chat);
        }
        CommandId::ClearContext => {
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
            // Clear on-disk messages
            if let Err(err) = state.backend.clear_messages(&chat.session.id) {
                push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                state.screen = Screen::Chat(chat);
                return;
            }
            // Reset TUI state
            chat.messages.clear();
            chat.live_assistant.clear();
            chat.live_reasoning.clear();
            chat.activity.clear();
            chat.activity_selected = 0;
            chat.scroll = 0;
            chat.total_input_tokens = 0;
            chat.total_output_tokens = 0;
            chat.last_total_tokens = 0;
            chat.context_limit = 0;
            chat.cost_usd = 0.0;
            chat.cache_read_tokens = 0;
            chat.cache_write_tokens = 0;
            chat.plan_title = None;
            chat.plan_steps.clear();
            chat.todos.clear();
            chat.find = None;
            chat.committed_approvals.clear();
            push_toast(
                state,
                ToastVariant::Success,
                "context cleared",
                Duration::from_secs(2),
            );
            state.screen = Screen::Chat(chat);
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
        .map_or("", |(p, _)| p)
        .to_ascii_lowercase();

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
        MODEL_PICKER_PROVIDER_PRIORITY
            .iter()
            .position(|&p| p == provider)
            .map_or(usize::MAX, |i| i + 1)
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
