use super::*;

use std::sync::Arc;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use rustcode_state::SessionStore;
use serde_json::Value;

use crate::LocalSessionBackend;

fn buffer_to_string(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push(buffer[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        out.push('\n');
    }
    out
}

#[test]
fn sessions_screen_renders_title_and_help() {
    let backend = TestBackend::new(60, 10);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
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
            skills: Vec::new(),
        },

        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
        tx: tokio::sync::mpsc::unbounded_channel().0,
        rx: tokio::sync::mpsc::unbounded_channel().1,
        request_seq: 0,
        last_area: Size {
            width: 60,
            height: 10,
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let buf = terminal.backend().buffer();
    let text = buffer_to_string(buf);
    assert!(text.contains("Sessions"), "text={text}");
    assert!(text.contains("Enter: open"), "text={text}");
    assert!(text.contains("s-1"), "text={text}");
}

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
    };

    let messages = vec![
        StoredMessage {
            id: "m-1".to_string(),
            role: MessageRole::User,
            created_at_unix_ms: 1,
            content: Value::String("hello".to_string()),
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
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
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
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },

        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Tool: read"), "text={text}");
    assert!(text.contains("ok=true"), "text={text}");
    assert!(text.contains("tool output"), "text={text}");
    assert!(text.contains("focus:composer"), "text={text}");
    assert!(text.contains("model:null"), "text={text}");
    assert!(text.contains("mode:agent"), "text={text}");
    assert!(!text.contains("Output:"), "text={text}");

    let mut state = state;
    if let Screen::Chat(chat) = &mut state.screen {
        chat.tool_details = true;
    }
    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Output:"), "text={text}");
    assert!(text.contains("tool output"), "text={text}");
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
    };

    let messages = vec![
        StoredMessage {
            id: "m-1".to_string(),
            role: MessageRole::System,
            created_at_unix_ms: 1,
            content: Value::String("## internal system prompt".to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        StoredMessage {
            id: "m-2".to_string(),
            role: MessageRole::User,
            created_at_unix_ms: 2,
            content: Value::String("hello".to_string()),
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
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
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
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },
        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(!text.contains("internal system prompt"), "text={text}");
    assert!(text.contains("hello"), "text={text}");
}

#[test]
fn approval_modal_renders_tool_name() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
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
            skills: Vec::new(),
        },

        pending_approval: Some(PendingApproval {
            request: ToolApprovalRequest {
                tool: "read".to_string(),
                permission: "read".to_string(),
                pattern: "README.md".to_string(),
                arguments: serde_json::json!({"path": "README.md"}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        }),
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Tool approval required"), "text={text}");
    assert!(text.contains("tool: read"), "text={text}");
    assert!(text.contains("target: README.md"), "text={text}");
    assert!(text.contains("Approve once"), "text={text}");
}

#[test]
fn approval_modal_shows_allow_all_edits_for_write() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
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
            skills: Vec::new(),
        },
        pending_approval: Some(PendingApproval {
            request: ToolApprovalRequest {
                tool: "write".to_string(),
                permission: "write".to_string(),
                pattern: "README.md".to_string(),
                arguments: serde_json::json!({"path": "README.md"}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        }),
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Approve all edits"), "text={text}");
}

#[test]
fn approval_modal_keeps_actions_visible_with_long_arguments() {
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let long_contents = "x".repeat(4000);
    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
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
            skills: Vec::new(),
        },
        pending_approval: Some(PendingApproval {
            request: ToolApprovalRequest {
                tool: "write".to_string(),
                permission: "write".to_string(),
                pattern: "big.py".to_string(),
                arguments: serde_json::json!({"path": "big.py", "contents": long_contents}),
                reason: "test".to_string(),
            },
            reply: reply_tx,
        }),
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Approve once"), "text={text}");
    assert!(text.contains("Approve all edits"), "text={text}");
}

#[test]
fn alt_tab_cycles_focus_in_chat() {
    let mut state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session: SessionInfo {
                id: "s-1".to_string(),
                title: Some("t1".to_string()),
                created_at_unix_ms: 0,
                updated_at_unix_ms: 0,
                parent_id: None,
                cwd: "/tmp".to_string(),
                workspace_root: "/tmp".to_string(),
                model: "null".to_string(),
            },
            messages: Vec::new(),
            scroll: 0,
            live_assistant: String::new(),
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
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
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },
        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Transcript);
}

#[test]
fn tab_does_not_change_focus_in_chat() {
    let mut state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session: SessionInfo {
                id: "s-1".to_string(),
                title: Some("t1".to_string()),
                created_at_unix_ms: 0,
                updated_at_unix_ms: 0,
                parent_id: None,
                cwd: "/tmp".to_string(),
                workspace_root: "/tmp".to_string(),
                model: "null".to_string(),
            },
            messages: Vec::new(),
            scroll: 0,
            live_assistant: String::new(),
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
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
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },
        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
    assert!(!handle_key(&mut state, key));

    let Screen::Chat(chat) = state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Composer);
}

#[test]
fn mouse_click_changes_chat_focus() {
    let mut state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
        sessions_view: vec![0],
        selected: 0,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen: Screen::Chat(ChatState {
            session: SessionInfo {
                id: "s-1".to_string(),
                title: Some("t1".to_string()),
                created_at_unix_ms: 0,
                updated_at_unix_ms: 0,
                parent_id: None,
                cwd: "/tmp".to_string(),
                workspace_root: "/tmp".to_string(),
                model: "null".to_string(),
            },
            messages: Vec::new(),
            scroll: 0,
            live_assistant: String::new(),
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
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
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },
        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    runtime::handle_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 110,
            row: 5,
            modifiers: KeyModifiers::NONE,
        },
    );
    let Screen::Chat(chat) = &state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Activity);

    runtime::handle_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 3,
            modifiers: KeyModifiers::NONE,
        },
    );
    let Screen::Chat(chat) = &state.screen else {
        panic!("expected chat screen")
    };
    assert_eq!(chat.focus, ChatFocus::Transcript);
}

#[test]
fn activity_details_modal_renders_tool_arguments() {
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
    };

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
            composer: String::new(),
            composer_cursor: 0,
            prompt_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            focus: ChatFocus::Activity,
            activity: vec![ActivityItem::ToolCall {
                id: "tc-1".to_string(),
                name: "read".to_string(),
                arguments: "{\"path\":\"README.md\"}".to_string(),
            }],
            activity_selected: 0,
            details_open: true,
            tool_details: false,
            find: None,
            running: None,
            pending_prompt: None,
            composer_cleared_by_ctrl_c: false,
            last_typing_time: None,
        }),
        status: None,
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults: InteractiveDefaults {
            workspace_root: std::path::PathBuf::from("/tmp"),
            model: "null".to_string(),
            skills: Vec::new(),
        },

        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
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
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Details"), "text={text}");
    assert!(text.contains("arguments:"), "text={text}");
    assert!(text.contains("README.md"), "text={text}");
}

#[test]
fn command_palette_renders_actions_and_search() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");

    let mut state = AppState {
        sessions: vec![SessionInfo {
            id: "s-1".to_string(),
            title: Some("t1".to_string()),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            parent_id: None,
            cwd: "/tmp".to_string(),
            workspace_root: "/tmp".to_string(),
            model: "null".to_string(),
        }],
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
            skills: Vec::new(),
        },
        pending_approval: None,
        submit_mode: InteractiveSubmitMode::Agent,
        backend: Arc::new(LocalSessionBackend::new(SessionStore::with_root(
            std::path::PathBuf::from("/tmp"),
        ))),
        config: None,
        executor: None,
        runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
        tx: tokio::sync::mpsc::unbounded_channel().0,
        rx: tokio::sync::mpsc::unbounded_channel().1,
        request_seq: 0,
        last_area: Size {
            width: 100,
            height: 24,
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell: None,
    };

    open_command_palette(&mut state);
    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let text = buffer_to_string(terminal.backend().buffer());
    assert!(text.contains("Commands"), "text={text}");
    assert!(text.contains("Search"), "text={text}");
    assert!(text.contains("New session"), "text={text}");
}
