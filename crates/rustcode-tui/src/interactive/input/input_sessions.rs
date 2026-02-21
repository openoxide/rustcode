use super::{
    build_prompt_history, compute_sessions_view, execute_command, open_command_palette, push_toast,
    AppState, ChatFocus, ChatState, CommandId, Duration, KeyCode, KeyEvent, KeyModifiers, Screen,
    ToastVariant,
};

pub(super) fn handle_sessions_key(state: &mut AppState, key: KeyEvent) -> bool {
    // Filter input mode: all chars go into the filter
    if state.sessions_filter_active {
        match key.code {
            KeyCode::Esc => {
                if state.sessions_filter.is_empty() {
                    state.sessions_filter_active = false;
                } else {
                    state.sessions_filter.clear();
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;
                }
            }
            KeyCode::Backspace => {
                state.sessions_filter.pop();
                state.sessions_view =
                    compute_sessions_view(&state.sessions, &state.sessions_filter);
                state.selected = state
                    .selected
                    .min(state.sessions_view.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(session) = state
                    .sessions_view
                    .get(state.selected)
                    .and_then(|idx| state.sessions.get(*idx))
                    .cloned()
                {
                    let messages = state
                        .backend
                        .load_messages(&session.id)
                        .unwrap_or_else(|err| {
                            state.status = Some(err.clone());
                            push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    let tokens_in = session.total_input_tokens;
                    let tokens_out = session.total_output_tokens;
                    let cost = session.cost_usd;
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
                        total_input_tokens: tokens_in,
                        total_output_tokens: tokens_out,
                        last_total_tokens: tokens_in + tokens_out,
                        context_limit: 0,
                        cost_usd: cost,
                        last_max_scroll: std::cell::Cell::new(0),
                        run_started_at: None,
                        last_run_elapsed: None,
                    });
                }
                state.sessions_filter_active = false;
            }
            KeyCode::Down => {
                if !state.sessions_view.is_empty() {
                    state.selected =
                        (state.selected + 1).min(state.sessions_view.len().saturating_sub(1));
                }
            }
            KeyCode::Up => state.selected = state.selected.saturating_sub(1),
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    state.sessions_filter.push(ch);
                    state.sessions_view =
                        compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;
                }
            }
            _ => {}
        }
        return false;
    }

    // Ctrl+key shortcuts — these work as long as filter mode is not active
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('p' | 'P') if ctrl => {
            open_command_palette(state, None);
        }
        KeyCode::Char('n' | 'N') if ctrl => {
            execute_command(state, CommandId::NewSession);
        }
        KeyCode::Char('e' | 'E') if ctrl => {
            execute_command(state, CommandId::RenameSession);
        }
        KeyCode::Char('d' | 'D') if ctrl => {
            execute_command(state, CommandId::DeleteSession);
        }
        KeyCode::Char('r' | 'R') if ctrl => {
            execute_command(state, CommandId::Refresh);
        }
        KeyCode::Char('q' | 'Q') | KeyCode::Esc if !ctrl => return true,
        KeyCode::Char('?') => state.help_open = true,
        KeyCode::Char('/') => state.sessions_filter_active = true,
        KeyCode::Down => {
            if !state.sessions_view.is_empty() {
                state.selected =
                    (state.selected + 1).min(state.sessions_view.len().saturating_sub(1));
            }
        }
        KeyCode::Up => {
            if state.selected > 0 {
                state.selected -= 1;
            }
        }
        KeyCode::PageDown => {
            if !state.sessions_view.is_empty() {
                state.selected =
                    (state.selected + 10).min(state.sessions_view.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => state.selected = state.selected.saturating_sub(10),
        KeyCode::Home => state.selected = 0,
        KeyCode::End => {
            state.selected = state.sessions_view.len().saturating_sub(1);
        }
        KeyCode::Enter => {
            state.status = None;
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
                .cloned()
            else {
                return false;
            };
            let messages = state
                .backend
                .load_messages(&session.id)
                .unwrap_or_else(|err| {
                    state.status = Some(err.clone());
                    push_toast(state, ToastVariant::Error, err, Duration::from_secs(4));
                    Vec::new()
                });
            let prompt_history = build_prompt_history(&messages);
            let tokens_in = session.total_input_tokens;
            let tokens_out = session.total_output_tokens;
            let cost = session.cost_usd;
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
                total_input_tokens: tokens_in,
                total_output_tokens: tokens_out,
                last_total_tokens: tokens_in + tokens_out,
                context_limit: 0,
                cost_usd: cost,
                last_max_scroll: std::cell::Cell::new(0),
                run_started_at: None,
                last_run_elapsed: None,
            });
        }
        _ => {}
    }
    false
}
