use std::io;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use rustcode_core::session::{MessageRole, SessionInfo, StoredMessage};
use rustcode_state::SessionStore;

use crate::TuiError;

enum Screen {
    Sessions,
    Transcript {
        session: SessionInfo,
        messages: Vec<StoredMessage>,
        scroll: u16,
    },
}

struct AppState {
    sessions: Vec<SessionInfo>,
    selected: usize,
    screen: Screen,
}

pub fn run_interactive(store: SessionStore) -> Result<(), TuiError> {
    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    execute!(stdout, EnterAlternateScreen).map_err(|err| TuiError::Io(err.to_string()))?;
    let _cleanup = TerminalCleanup;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|err| TuiError::Io(err.to_string()))?;
    terminal
        .clear()
        .map_err(|err| TuiError::Io(err.to_string()))?;

    let mut state = AppState {
        sessions: store
            .list_sessions()
            .map_err(|err| TuiError::State(err.to_string()))?,
        selected: 0,
        screen: Screen::Sessions,
    };

    loop {
        terminal
            .draw(|frame| render(frame, &state))
            .map_err(|err| TuiError::Io(err.to_string()))?;

        if event::poll(Duration::from_millis(100)).map_err(|err| TuiError::Io(err.to_string()))? {
            let evt = event::read().map_err(|err| TuiError::Io(err.to_string()))?;
            match evt {
                CEvent::Key(key) if key.kind == KeyEventKind::Press => match &mut state.screen {
                    Screen::Sessions => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Down => {
                            if !state.sessions.is_empty() {
                                state.selected = (state.selected + 1)
                                    .min(state.sessions.len().saturating_sub(1));
                            }
                        }
                        KeyCode::Up => {
                            if state.selected > 0 {
                                state.selected -= 1;
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(session) = state.sessions.get(state.selected).cloned() {
                                let messages = store
                                    .load_messages(&session.id)
                                    .map_err(|err| TuiError::State(err.to_string()))?;
                                state.screen = Screen::Transcript {
                                    session,
                                    messages,
                                    scroll: 0,
                                };
                            }
                        }
                        KeyCode::Char('r') => {
                            state.sessions = store
                                .list_sessions()
                                .map_err(|err| TuiError::State(err.to_string()))?;
                            state.selected =
                                state.selected.min(state.sessions.len().saturating_sub(1));
                        }
                        _ => {}
                    },
                    Screen::Transcript {
                        session,
                        messages,
                        scroll,
                    } => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            state.screen = Screen::Sessions;
                        }
                        KeyCode::Down => {
                            *scroll = scroll.saturating_add(1);
                        }
                        KeyCode::Up => {
                            *scroll = scroll.saturating_sub(1);
                        }
                        KeyCode::Char('r') => {
                            let refreshed = store
                                .load_messages(&session.id)
                                .map_err(|err| TuiError::State(err.to_string()))?;
                            *messages = refreshed;
                        }
                        _ => {}
                    },
                },
                CEvent::Resize(_w, _h) => {}
                _ => {}
            }
        }
    }
}

fn render(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    match &state.screen {
        Screen::Sessions => render_sessions(frame, state),
        Screen::Transcript {
            session,
            messages,
            scroll,
        } => render_transcript(frame, session, messages, *scroll),
    }
}

fn render_sessions(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(frame.area());

    let items = if state.sessions.is_empty() {
        vec![ListItem::new("(no sessions)")]
    } else {
        state
            .sessions
            .iter()
            .map(|session| {
                let title = session.title.as_deref().unwrap_or("-");
                ListItem::new(format!("{}\t{}", session.id, title))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(Block::default().title("Sessions").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ratatui::widgets::ListState::default();
    if !state.sessions.is_empty() {
        list_state.select(Some(state.selected));
    }
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let help = Paragraph::new(Line::from(vec![
        Span::raw("Up/Down: select  "),
        Span::raw("Enter: open  "),
        Span::raw("r: refresh  "),
        Span::raw("q: quit"),
    ]))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(help, chunks[1]);
}

fn render_transcript(
    frame: &mut ratatui::Frame<'_>,
    session: &SessionInfo,
    messages: &[StoredMessage],
    scroll: u16,
) {
    let title = session
        .title
        .as_deref()
        .unwrap_or(session.id.as_str())
        .to_string();
    let mut lines = Vec::new();
    for msg in messages {
        let role = match msg.role {
            MessageRole::System => "System",
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
            MessageRole::Tool => "Tool",
        };
        let content = msg.content.as_str().unwrap_or("<non-string>");
        lines.push(Line::from(vec![
            Span::styled(role, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": "),
            Span::raw(content),
        ]));
        lines.push(Line::raw(""));
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, frame.area());
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = stdout.execute(LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    use super::*;

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
            selected: 0,
            screen: Screen::Sessions,
        };
        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let buf = terminal.backend().buffer();
        let text = buffer_to_string(buf);
        assert!(text.contains("Sessions"), "text={text}");
        assert!(text.contains("Enter: open"), "text={text}");
        assert!(text.contains("s-1"), "text={text}");
    }
}
