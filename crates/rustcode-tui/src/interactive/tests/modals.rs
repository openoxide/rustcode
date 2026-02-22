use std::collections::VecDeque;
use std::sync::Arc;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rustcode_state::SessionStore;

use super::super::*;
use super::buffer_to_string;
use crate::LocalSessionBackend;

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

#[test]
fn activity_details_modal_renders_tool_arguments() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let session = make_session();

    let state = AppState {
        sessions: vec![session.clone()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session,
            messages: Vec::new(),
            scroll: 0,
            live_assistant: String::new(),
            live_reasoning: String::new(),
            show_reasoning: false,
            composer: String::new(),
            composer_cursor: 0,
            paste_buffer: None,
            prompt_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            focus: ChatFocus::Activity,
            activity: VecDeque::from([ActivityItem::ToolCall {
                id: "tc-1".to_string(),
                name: "read".to_string(),
                arguments: "{\"path\":\"README.md\"}".to_string(),
            }]),
            activity_selected: 0,
            details_open: true,
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
        }),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Details"), "text={text}");
    assert!(text.contains("arguments:"), "text={text}");
    assert!(text.contains("README.md"), "text={text}");
}

#[test]
fn command_palette_renders_actions_and_search() {
    let backend = TestBackend::new(100, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let mut state = AppState {
        sessions: vec![make_session()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Sessions,
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
            width: 100,
            height: 40,
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
        git_stat: None,
    };

    open_command_palette(&mut state, None);
    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Commands"), "text={text}");
    assert!(text.contains("Search"), "text={text}");
    assert!(text.contains("New session"), "text={text}");
}
