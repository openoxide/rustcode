use super::*;

use std::sync::Arc;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use rustcode_state::SessionStore;

use crate::LocalSessionBackend;

mod approval_modal;
mod chat_render;
mod focus;
mod modals;

/// Create a no-op [`FrameRequester`] for tests that don't need frame scheduling.
fn test_frame_requester() -> super::runtime::frame_scheduler::FrameRequester {
    super::runtime::frame_scheduler::test_frame_requester()
}

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
            branch: "main".to_string(),
            total_input_tokens: 0,
            total_output_tokens: 0,
            cost_usd: 0.0,
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
        frame_requester: test_frame_requester(),
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
        git_stat: None,
    };

    terminal.draw(|frame| render(frame, &state)).expect("draw");
    let buf = terminal.backend().buffer();
    let text = buffer_to_string(buf);
    assert!(text.contains("Sessions"), "text={text}");
    // Footer shows key hints (filter mode = false, so shows command-palette hints)
    assert!(
        text.contains("Ctrl+P") || text.contains("filter"),
        "text={text}"
    );
    assert!(text.contains("s-1"), "text={text}");
    assert!(text.contains("[main]"), "text={text}");
}
