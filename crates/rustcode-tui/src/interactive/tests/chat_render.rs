use std::collections::VecDeque;
use std::sync::Arc;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use rustcode_state::{PromptHistoryStore, SessionStore};
use serde_json::Value;

use super::super::*;
use super::buffer_to_string;
use crate::LocalSessionBackend;

#[test]
fn chat_screen_renders_tool_messages_and_toggle_label() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let session = SessionInfo {
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
    };

    let messages = vec![
        StoredMessage {
            id: "m-1".to_string(),
            role: MessageRole::User,
            created_at_unix_ms: 1,
            content: Value::String("hello".to_string()),
            reasoning: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        StoredMessage {
            id: "m-2".to_string(),
            role: MessageRole::Tool,
            created_at_unix_ms: 2,
            content: Value::String(
                "{\"ok\":true,\"truncated\":false,\"output\":\"tool output\"}".to_string(),
            ),
            reasoning: None,
            tool_call_id: Some("tc-1".to_string()),
            tool_name: Some("read".to_string()),
            tool_calls: Vec::new(),
        },
    ];

    let state = AppState {
        sessions: vec![session.clone()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session,
            messages,
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
            last_max_scroll: std::cell::Cell::new(0),
                last_transcript_wrapped_count: std::cell::Cell::new(0),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    // Collapsed mode (tool_details=false): tool calls show as batch summary
    assert!(text.contains("read"), "text={text}");
    assert!(text.contains("(ctrl+o to expand)"), "text={text}");
    assert!(text.contains("model:null"), "text={text}");
    assert!(!text.contains("[Null]"), "text={text}"); // null provider is hidden
                                                      // "ok=true" and "Output:" were removed in the new compact rendering
    assert!(!text.contains("ok=true"), "text={text}");
    assert!(!text.contains("Output:"), "text={text}");

    let mut state = state;
    if let Screen::Chat(chat) = &mut state.screen {
        chat.tool_details = true;
        chat.output_details = true;
    }
    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    // Expanded mode: tool name and output are shown
    assert!(text.contains("read"), "text={text}");
    assert!(text.contains("Read 1 line"), "text={text}");
}

#[test]
fn chat_screen_hides_system_messages() {
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let session = SessionInfo {
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
    };

    let messages = vec![
        StoredMessage {
            id: "m-1".to_string(),
            role: MessageRole::System,
            created_at_unix_ms: 1,
            content: Value::String("## internal system prompt".to_string()),
            reasoning: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        StoredMessage {
            id: "m-2".to_string(),
            role: MessageRole::User,
            created_at_unix_ms: 2,
            content: Value::String("hello".to_string()),
            reasoning: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
    ];

    let state = AppState {
        sessions: vec![session.clone()],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session,
            messages,
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
            last_max_scroll: std::cell::Cell::new(0),
                last_transcript_wrapped_count: std::cell::Cell::new(0),
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
            height: 20,
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
        git_stat: None,
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(!text.contains("internal system prompt"), "text={text}");
    assert!(text.contains("hello"), "text={text}");
}
