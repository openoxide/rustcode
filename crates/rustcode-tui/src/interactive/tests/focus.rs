use std::collections::VecDeque;
use std::sync::Arc;

use rustcode_state::{PromptHistoryStore, SessionStore};

use super::super::*;
use crate::LocalSessionBackend;

fn make_chat_state_with_focus(session: SessionInfo, focus: ChatFocus) -> ChatState {
    ChatState {
        session,
        messages: Vec::new(),
        scroll: 0,
        live_assistant: String::new(),
        live_reasoning: String::new(),
        show_reasoning: false,
        composer: String::new(),
        composer_cursor: 0,
        paste_buffer: None,
        focus,
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
        last_max_scroll: std::cell::Cell::new(0),
        run_started_at: None,
        last_run_elapsed: None,
        plan_title: None,
        plan_steps: Vec::new(),
        todos: Vec::new(),
    }
}

fn make_session() -> SessionInfo {
    SessionInfo {
        id: "s-1".to_string(),
        title: Some("t1".to_string()),
        created_at_unix_ms: 0,
        updated_at_unix_ms: 0,
        parent_id: None,
        cwd: "/tmp".to_string(),
        workspace_root: "/tmp".to_string(),
        model: "null".to_string(),
        branch: String::new(),
        total_input_tokens: 0,
        total_output_tokens: 0,
        cost_usd: 0.0,
    }
}

fn make_focus_state(session: SessionInfo) -> AppState {
    AppState {
        sessions: vec![session.clone()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(make_chat_state_with_focus(session, ChatFocus::Composer)),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            provider: String::new(),
            skills: Vec::new(),
        },
        prompt_history_store: PromptHistoryStore::with_path(std::path::PathBuf::from(
            "/tmp/rustcode-test-prompt-history.json",
        )),
        global_prompt_history: Vec::new(),
        history_cursor: None,
        history_draft: String::new(),
        pending_approval: None,
        approval_selection: 0,
        approval_mode: ApprovalMode::Normal,
        mode_flag: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        frame_requester: super::test_frame_requester(),
        tx: tokio::sync::mpsc::unbounded_channel().0,
        rx: tokio::sync::mpsc::unbounded_channel().1,
        request_seq: 0,
        last_area: Size {
            width: 120,
            height: 30,
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
        git_stat: None,
    }
}

#[test]
fn alt_tab_cycles_focus_in_chat() {
    let mut state = make_focus_state(make_session());

    // Alt+Tab with activity_hidden=true: focus stays at Composer (no-op)
    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = &state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Composer);

    // Alt+Tab with activity visible: should move to Activity
    if let Screen::Chat(chat) = &mut state.screen {
        chat.activity_hidden = false;
    }
    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Activity);
}

#[test]
fn tab_does_not_change_focus_in_chat() {
    let mut state = make_focus_state(make_session());

    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Composer);
}

#[test]
fn shift_enter_inserts_newline_in_composer() {
    let mut state = make_focus_state(make_session());
    if let Screen::Chat(chat) = &mut state.screen {
        chat.composer = "hello".to_string();
        chat.composer_cursor = chat.composer.len();
    }

    let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.composer, "hello\n");
    assert_eq!(chat.composer_cursor, chat.composer.len());
}
