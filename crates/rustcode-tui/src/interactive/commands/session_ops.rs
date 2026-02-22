use std::collections::VecDeque;

use rustcode_core::SessionInfo;

use super::super::{
    build_prompt_history, compute_sessions_view, push_toast, sort_sessions, AppState, ChatFocus,
    ChatState, Duration, Screen, ToastVariant,
};

pub(crate) fn refresh_chat_messages(state: &mut AppState, chat: &mut ChatState) {
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
pub(crate) fn open_session_by_id(state: &mut AppState, session_id: &str) {
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

pub(crate) fn open_session(state: &mut AppState, session: SessionInfo) {
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
        context_limit: 0,
        last_max_scroll: std::cell::Cell::new(0),
        run_started_at: None,
        last_run_elapsed: None,
        plan_title: None,
        plan_steps: Vec::new(),
        todos: Vec::new(),
    });
    if let Some(idx) = state.sessions.iter().position(|s| s.id == session_id) {
        state.selected = idx;
    }
}
