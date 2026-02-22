use std::collections::VecDeque;
use std::sync::Arc;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rustcode_state::{PromptHistoryStore, SessionStore};

use super::super::*;
use super::buffer_to_string;
use crate::LocalSessionBackend;

fn make_chat_state(session: SessionInfo) -> ChatState {
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

fn make_state(session: SessionInfo, pending: PendingApproval, width: u16, height: u16) -> AppState {
    AppState {
        sessions: vec![session.clone()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(make_chat_state(session)),
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
        pending_approval: Some(pending),
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
        last_area: Size { width, height },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
        git_stat: None,
    }
}

#[test]
fn approval_modal_renders_tool_name() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = make_state(
        make_session(),
        PendingApproval {
            request: ToolApprovalRequest {
                tool: "read".to_string(),
                permission: "read".to_string(),
                pattern: "README.md".to_string(),
                arguments: serde_json::json!({"path": "README.md"}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        },
        120,
        30,
    );

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    // Inline approval: "Approval" in selector title, tool name in transcript,
    // and an explicit once option with tool name in brackets.
    assert!(text.contains("Approval"), "text={text}");
    assert!(text.contains("read"), "text={text}"); // tool name in transcript
    assert!(!text.contains("Reason:"), "text={text}"); // reason line hidden
    assert!(text.contains("Approve once [read]"), "text={text}"); // option in selector
}

#[test]
fn approval_modal_shows_allow_all_tools_in_directory_for_write() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = make_state(
        make_session(),
        PendingApproval {
            request: ToolApprovalRequest {
                tool: "write".to_string(),
                permission: "write".to_string(),
                pattern: "README.md".to_string(),
                arguments: serde_json::json!({"path": "README.md"}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        },
        120,
        30,
    );

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    // Inline approval selector for write: auto-pilot option is visible.
    assert!(
        text.contains("Approve all tools (auto-pilot mode)"),
        "text={text}"
    );
}

#[test]
fn approval_modal_keeps_actions_visible_with_long_arguments() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let long_contents = "x".repeat(4000);
    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = make_state(
        make_session(),
        PendingApproval {
            request: ToolApprovalRequest {
                tool: "write".to_string(),
                permission: "write".to_string(),
                pattern: "big.py".to_string(),
                arguments: serde_json::json!({"path": "big.py", "contents": long_contents}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        },
        120,
        30,
    );

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    // Inline selector: both options always visible regardless of content preview length
    // (selector is in a fixed-height composer area, not inline with preview)
    assert!(text.contains("Approve once [write]"), "text={text}");
    assert!(
        text.contains("Approve all tools (auto-pilot mode)"),
        "text={text}"
    );
}

#[test]
fn approval_modal_shows_bash_and_autopilot_options_for_bash() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = make_state(
        make_session(),
        PendingApproval {
            request: ToolApprovalRequest {
                tool: "bash".to_string(),
                permission: "bash".to_string(),
                pattern: "echo hi".to_string(),
                arguments: serde_json::json!({"command":"echo hi"}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        },
        120,
        30,
    );

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Approve once [bash]"), "text={text}");
    assert!(
        text.contains("Allow all edits"),
        "text={text}"
    );
    assert!(
        text.contains("Approve all tools (auto-pilot mode)"),
        "text={text}"
    );
}
