use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, ExecutableCommand};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use tokio_util::sync::CancellationToken;

use rustcode_core::command::AgentOptions;
use rustcode_core::event::EventPayload;
use rustcode_core::ports::EventPublisher;
use rustcode_core::{Command, CommandContext, ResolvedConfig, SessionInfo};
use rustcode_core::{MessageRole, SessionMeta, StoredMessage, ToolApprovalRequest};
use rustcode_state::SessionStore;

use crate::{
    InteractiveDefaults, InteractiveMsg, InteractiveServices, InteractiveStart, TuiError,
    TuiPublisher,
};

enum Screen {
    Sessions,
    Chat(ChatState),
}

struct PendingApproval {
    request: ToolApprovalRequest,
    reply: tokio::sync::oneshot::Sender<bool>,
}

struct RunningCommand {
    cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatFocus {
    Composer,
    Transcript,
    Activity,
}

#[derive(Debug, Clone)]
enum ActivityItem {
    CommandAccepted { name: String },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        output: String,
    },
    OutputChunk { text: String },
    Warning { message: String },
    Failure { message: String },
    Completed,
}

impl ActivityItem {
    fn title(&self) -> String {
        match self {
            ActivityItem::CommandAccepted { name } => format!("command: {name}"),
            ActivityItem::ToolCall { name, .. } => format!("tool: {name}"),
            ActivityItem::ToolResult { name, ok, .. } => format!("tool result: {name} ok={ok}"),
            ActivityItem::OutputChunk { .. } => "assistant".to_string(),
            ActivityItem::Warning { .. } => "warning".to_string(),
            ActivityItem::Failure { .. } => "failure".to_string(),
            ActivityItem::Completed => "completed".to_string(),
        }
    }

    fn summary(&self) -> String {
        let mut s = match self {
            ActivityItem::CommandAccepted { name } => name.clone(),
            ActivityItem::ToolCall { id, name, .. } => format!("{name} ({id})"),
            ActivityItem::ToolResult { id, name, ok, .. } => format!("{name} ({id}) ok={ok}"),
            ActivityItem::OutputChunk { text } => text.replace('\n', " "),
            ActivityItem::Warning { message } => message.clone(),
            ActivityItem::Failure { message } => message.clone(),
            ActivityItem::Completed => "done".to_string(),
        };
        if s.len() > 140 {
            s.truncate(140);
            s.push_str("...");
        }
        s
    }

    fn details_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        out.push(Line::from(vec![Span::styled(
            self.title(),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        out.push(Line::raw(""));
        match self {
            ActivityItem::CommandAccepted { name } => {
                out.push(Line::raw(format!("name: {name}")));
            }
            ActivityItem::ToolCall {
                id,
                name,
                arguments,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(""));
                out.push(Line::raw("arguments:"));
                let pretty = serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| arguments.clone());
                for line in pretty.lines().take(32) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::ToolResult {
                id,
                name,
                ok,
                output,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(format!("ok: {ok}")));
                out.push(Line::raw(""));
                out.push(Line::raw("output:"));
                let pretty = serde_json::from_str::<serde_json::Value>(output)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| output.clone());
                for line in pretty.lines().take(48) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::OutputChunk { text } => {
                for line in text.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Warning { message } => {
                for line in message.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Failure { message } => {
                for line in message.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Completed => {}
        }

        out
    }
}

struct ChatState {
    session: SessionInfo,
    messages: Vec<StoredMessage>,
    scroll: u16,
    composer: String,
    prompt_history: Vec<String>,
    history_cursor: Option<usize>,
    history_draft: String,
    focus: ChatFocus,
    activity: Vec<ActivityItem>,
    activity_selected: usize,
    details_open: bool,
    tool_details: bool,
    running: Option<RunningCommand>,
}

fn build_prompt_history(messages: &[StoredMessage]) -> Vec<String> {
    let mut out = Vec::new();
    for msg in messages {
        if msg.role != MessageRole::User {
            continue;
        }
        let Some(text) = msg.content.as_str() else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if out.last().is_some_and(|last| last == trimmed) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    while out.len() > 200 {
        out.remove(0);
    }
    out
}

struct AppState {
    sessions: Vec<SessionInfo>,
    sessions_view: Vec<usize>,
    selected: usize,
    sessions_filter: String,
    sessions_filter_active: bool,
    screen: Screen,
    status: Option<String>,
    defaults: InteractiveDefaults,

    pending_approval: Option<PendingApproval>,

    store: SessionStore,
    config: Option<Arc<ResolvedConfig>>,
    executor: Option<Arc<dyn rustcode_core::ports::CommandExecutor>>,
    runtime: tokio::runtime::Handle,
    tx: tokio::sync::mpsc::UnboundedSender<InteractiveMsg>,
    rx: tokio::sync::mpsc::UnboundedReceiver<InteractiveMsg>,
    request_seq: u64,
}

fn compute_sessions_view(sessions: &[SessionInfo], filter: &str) -> Vec<usize> {
    if filter.trim().is_empty() {
        return (0..sessions.len()).collect();
    }
    let needle = filter.to_ascii_lowercase();
    let mut out = Vec::new();
    for (idx, session) in sessions.iter().enumerate() {
        let title = session.title.as_deref().unwrap_or("");
        let haystack = format!("{}\t{}", session.id, title).to_ascii_lowercase();
        if haystack.contains(&needle) {
            out.push(idx);
        }
    }
    out
}

pub fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    let InteractiveServices {
        store,
        defaults,
        initial_status,
        start,
        runtime,
        handles,
        config,
        executor,
    } = services;

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    execute!(stdout, EnterAlternateScreen).map_err(|err| TuiError::Io(err.to_string()))?;
    let _cleanup = TerminalCleanup;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|err| TuiError::Io(err.to_string()))?;
    terminal
        .clear()
        .map_err(|err| TuiError::Io(err.to_string()))?;

    let sessions = store
        .list_sessions()
        .map_err(|err| TuiError::State(err.to_string()))?;
    let sessions_view = compute_sessions_view(&sessions, "");
    let mut selected = 0usize;
    let mut screen = Screen::Sessions;
    let mut auto_submit = None;

    match start {
        InteractiveStart::Sessions => {}
        InteractiveStart::Chat {
            session,
            prompt,
            auto_submit: should_submit,
        } => {
            if let Some(idx) = sessions.iter().position(|candidate| candidate.id == session.id) {
                selected = idx;
            }
            let messages = store
                .load_messages(&session.id)
                .map_err(|err| TuiError::State(err.to_string()))?;
            let prompt_history = build_prompt_history(&messages);
            screen = Screen::Chat(ChatState {
                session,
                messages,
                scroll: 0,
                composer: if should_submit {
                    String::new()
                } else {
                    prompt.clone().unwrap_or_default()
                },
                prompt_history,
                history_cursor: None,
                history_draft: String::new(),
                focus: ChatFocus::Composer,
                activity: Vec::new(),
                activity_selected: 0,
                details_open: false,
                tool_details: false,
                running: None,
            });
            if should_submit {
                auto_submit = prompt;
            }
        }
    }

    let mut state = AppState {
        sessions,
        sessions_view,
        selected,
        sessions_filter: String::new(),
        sessions_filter_active: false,
        screen,
        status: initial_status,
        defaults,

        pending_approval: None,
        store,
        config,
        executor,
        runtime,
        tx: handles.tx,
        rx: handles.rx,
        request_seq: 0,
    };

    if let Some(prompt) = auto_submit {
        if !prompt.trim().is_empty() {
            start_auto_submit(&mut state, prompt);
        }
    }

    loop {
        drain_messages(&mut state);

        terminal
            .draw(|frame| render(frame, &state))
            .map_err(|err| TuiError::Io(err.to_string()))?;

        if event::poll(Duration::from_millis(50)).map_err(|err| TuiError::Io(err.to_string()))? {
            let evt = event::read().map_err(|err| TuiError::Io(err.to_string()))?;
            if let CEvent::Key(key) = evt {
                if key.kind == KeyEventKind::Press {
                    if handle_key(&mut state, key) {
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn start_auto_submit(state: &mut AppState, prompt: String) {
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    submit_prompt(state, &mut chat, prompt);
    state.screen = Screen::Chat(chat);
}

fn drain_messages(state: &mut AppState) {
    loop {
        let msg = match state.rx.try_recv() {
            Ok(msg) => msg,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
        };

        let mut screen = std::mem::replace(&mut state.screen, Screen::Sessions);
        match msg {
            InteractiveMsg::EngineEvent(event) => {
                if let Screen::Chat(chat) = &mut screen {
                    let mut refresh = false;
                    let item = match &event.payload {
                        EventPayload::CommandAccepted { name } => {
                            Some(ActivityItem::CommandAccepted { name: name.clone() })
                        }
                        EventPayload::ToolCall {
                            id,
                            name,
                            arguments,
                        } => Some(ActivityItem::ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            arguments: arguments.clone(),
                        }),
                        EventPayload::ToolResult {
                            id,
                            name,
                            ok,
                            output,
                        } => Some(ActivityItem::ToolResult {
                            id: id.clone(),
                            name: name.clone(),
                            ok: *ok,
                            output: output.clone(),
                        }),
                        EventPayload::OutputChunk { text } => {
                            Some(ActivityItem::OutputChunk { text: text.clone() })
                        }
                        EventPayload::Warning { message } => {
                            Some(ActivityItem::Warning { message: message.clone() })
                        }
                        EventPayload::Failure { message } => {
                            state.status = Some(message.clone());
                            refresh = true;
                            Some(ActivityItem::Failure {
                                message: message.clone(),
                            })
                        }
                        EventPayload::Completed => {
                            chat.running = None;
                            refresh = true;
                            Some(ActivityItem::Completed)
                        }
                        EventPayload::ServeRequest { .. } => None,
                    };

                    if let Some(item) = item {
                        chat.activity.push(item);
                    }

                    while chat.activity.len() > 200 {
                        chat.activity.remove(0);
                        if chat.activity_selected > 0 {
                            chat.activity_selected -= 1;
                        }
                    }
                    if chat.activity_selected >= chat.activity.len() {
                        chat.activity_selected = chat.activity.len().saturating_sub(1);
                    }

                    if refresh {
                        match state.store.load_messages(&chat.session.id) {
                            Ok(messages) => chat.messages = messages,
                            Err(err) => {
                                state.status = Some(format!("failed to load transcript: {err}"));
                            }
                        }
                    }
                }
            }
            InteractiveMsg::ApprovalRequest { request, reply } => {
                // Always present approvals globally; the run might be streaming while the user
                // navigates around.
                state.pending_approval = Some(PendingApproval { request, reply });
            }
            InteractiveMsg::RunEnded { ok, message } => {
                if let Screen::Chat(chat) = &mut screen {
                    chat.running = None;
                    if ok {
                        state.status = None;
                    } else {
                        state.status = message;
                    }
                    match state.store.load_messages(&chat.session.id) {
                        Ok(messages) => chat.messages = messages,
                        Err(err) => {
                            state.status = Some(format!("failed to load transcript: {err}"));
                        }
                    }
                }
            }
        }

        state.screen = screen;
    }
}

enum ChatNav {
    Stay,
    ToSessions,
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> bool {
    if let Some(pending) = state.pending_approval.take() {
        let decision = match key.code {
            KeyCode::Char('a') => Some(true),
            KeyCode::Char('d') => Some(false),
            KeyCode::Esc | KeyCode::Char('q') => Some(false),
            _ => None,
        };
        if let Some(decision) = decision {
            let _ = pending.reply.send(decision);
            state.status = Some(format!(
                "approval: tool={} decision={decision}",
                pending.request.tool
            ));
        } else {
            state.pending_approval = Some(pending);
        }
        return false;
    }

    match &state.screen {
        Screen::Sessions => handle_sessions_key(state, key),
        Screen::Chat(_) => {
            let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions)
            else {
                return false;
            };

            let nav = handle_chat_key(state, &mut chat, key);
            match nav {
                ChatNav::Stay => {
                    state.screen = Screen::Chat(chat);
                }
                ChatNav::ToSessions => {
                    state.screen = Screen::Sessions;
                }
            }
            false
        }
    }
}

fn handle_sessions_key(state: &mut AppState, key: KeyEvent) -> bool {
    if state.sessions_filter_active {
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc => {
                if state.sessions_filter.is_empty() {
                    state.sessions_filter_active = false;
                } else {
                    state.sessions_filter.clear();
                    state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;
                }
            }
            KeyCode::Backspace => {
                state.sessions_filter.pop();
                state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
                state.selected = state.selected.min(state.sessions_view.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(session) = state
                    .sessions_view
                    .get(state.selected)
                    .and_then(|idx| state.sessions.get(*idx))
                    .cloned()
                {
                    let messages = state
                        .store
                        .load_messages(&session.id)
                        .map_err(|err| TuiError::State(err.to_string()))
                        .unwrap_or_else(|err| {
                            state.status = Some(err.to_string());
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    state.screen = Screen::Chat(ChatState {
                        session,
                        messages,
                        scroll: 0,
                        composer: String::new(),
                        prompt_history,
                        history_cursor: None,
                        history_draft: String::new(),
                        focus: ChatFocus::Composer,
                        activity: Vec::new(),
                        activity_selected: 0,
                        details_open: false,
                        tool_details: false,
                        running: None,
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
            KeyCode::Up => {
                state.selected = state.selected.saturating_sub(1);
            }
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

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('/') => {
            state.sessions_filter_active = true;
        }
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
        KeyCode::Char('r') => {
            state.status = None;
            state.sessions = state
                .store
                .list_sessions()
                .map_err(|err| TuiError::State(err.to_string()))
                .unwrap_or_else(|err| {
                    state.status = Some(err.to_string());
                    Vec::new()
                });
            state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
            state.selected = state.selected.min(state.sessions_view.len().saturating_sub(1));
        }
        KeyCode::Char('n') => {
            state.status = None;
            state.sessions_filter.clear();
            state.sessions_filter_active = false;
            let cwd = match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(err) => {
                    state.status = Some(format!("failed to resolve cwd: {err}"));
                    return false;
                }
            };
            match state.store.create_session(
                None,
                None,
                &cwd,
                state.defaults.workspace_root.as_path(),
                state.defaults.model.as_str(),
            ) {
                Ok(session) => {
                    state.sessions = state
                        .store
                        .list_sessions()
                        .map_err(|err| TuiError::State(err.to_string()))
                        .unwrap_or_else(|err| {
                            state.status = Some(err.to_string());
                            Vec::new()
                        });
                    state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;

                    let messages = state
                        .store
                        .load_messages(&session.id)
                        .map_err(|err| TuiError::State(err.to_string()))
                        .unwrap_or_else(|err| {
                            state.status = Some(err.to_string());
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    state.screen = Screen::Chat(ChatState {
                        session,
                        messages,
                        scroll: 0,
                        composer: String::new(),
                        prompt_history,
                        history_cursor: None,
                        history_draft: String::new(),
                        focus: ChatFocus::Composer,
                        activity: Vec::new(),
                        activity_selected: 0,
                        details_open: false,
                        tool_details: false,
                        running: None,
                    });
                }
                Err(err) => {
                    state.status = Some(format!("failed to create session: {err}"));
                }
            }
        }
        KeyCode::Char('f') => {
            state.status = None;
            state.sessions_filter.clear();
            state.sessions_filter_active = false;
            let Some(session) = state
                .sessions_view
                .get(state.selected)
                .and_then(|idx| state.sessions.get(*idx))
            else {
                return false;
            };
            match state.store.fork_session(&session.id, None) {
                Ok(forked) => {
                    state.sessions = state
                        .store
                        .list_sessions()
                        .map_err(|err| TuiError::State(err.to_string()))
                        .unwrap_or_else(|err| {
                            state.status = Some(err.to_string());
                            Vec::new()
                        });
                    state.sessions_view = compute_sessions_view(&state.sessions, &state.sessions_filter);
                    state.selected = 0;

                    let messages = state
                        .store
                        .load_messages(&forked.id)
                        .map_err(|err| TuiError::State(err.to_string()))
                        .unwrap_or_else(|err| {
                            state.status = Some(err.to_string());
                            Vec::new()
                        });
                    let prompt_history = build_prompt_history(&messages);
                    state.screen = Screen::Chat(ChatState {
                        session: forked,
                        messages,
                        scroll: 0,
                        composer: String::new(),
                        prompt_history,
                        history_cursor: None,
                        history_draft: String::new(),
                        focus: ChatFocus::Composer,
                        activity: Vec::new(),
                        activity_selected: 0,
                        details_open: false,
                        tool_details: false,
                        running: None,
                    });
                }
                Err(err) => {
                    state.status = Some(format!("failed to fork session: {err}"));
                }
            }
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
                .store
                .load_messages(&session.id)
                .map_err(|err| TuiError::State(err.to_string()))
                .unwrap_or_else(|err| {
                    state.status = Some(err.to_string());
                    Vec::new()
                });
            let prompt_history = build_prompt_history(&messages);
            state.screen = Screen::Chat(ChatState {
                session,
                messages,
                scroll: 0,
                composer: String::new(),
                prompt_history,
                history_cursor: None,
                history_draft: String::new(),
                focus: ChatFocus::Composer,
                activity: Vec::new(),
                activity_selected: 0,
                details_open: false,
                tool_details: false,
                running: None,
            });
        }
        _ => {}
    }
    false
}

fn handle_chat_key(state: &mut AppState, chat: &mut ChatState, key: KeyEvent) -> ChatNav {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        if let Some(running) = &chat.running {
            running.cancellation.cancel();
            chat.activity.push(ActivityItem::Warning {
                message: "cancel requested".to_string(),
            });
        }
        return ChatNav::Stay;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('n')) {
        if chat.running.is_some() {
            state.status = Some("cannot create session while running".to_string());
            return ChatNav::Stay;
        }
        let cwd = match std::env::current_dir() {
            Ok(cwd) => cwd,
            Err(err) => {
                state.status = Some(format!("failed to resolve cwd: {err}"));
                return ChatNav::Stay;
            }
        };
        match state.store.create_session(
            None,
            None,
            &cwd,
            state.defaults.workspace_root.as_path(),
            state.defaults.model.as_str(),
        ) {
            Ok(session) => {
                let messages = state
                    .store
                    .load_messages(&session.id)
                    .map_err(|err| TuiError::State(err.to_string()))
                    .unwrap_or_else(|err| {
                        state.status = Some(err.to_string());
                        Vec::new()
                    });
                chat.session = session;
                chat.messages = messages;
                chat.scroll = 0;
                chat.composer.clear();
                chat.prompt_history = build_prompt_history(&chat.messages);
                chat.history_cursor = None;
                chat.history_draft.clear();
                chat.focus = ChatFocus::Composer;
                chat.activity.clear();
                chat.activity_selected = 0;
                chat.details_open = false;
                chat.tool_details = false;
                chat.running = None;
                state.status = None;
            }
            Err(err) => {
                state.status = Some(format!("failed to create session: {err}"));
            }
        }
        return ChatNav::Stay;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('f')) {
        if chat.running.is_some() {
            state.status = Some("cannot fork session while running".to_string());
            return ChatNav::Stay;
        }
        match state.store.fork_session(&chat.session.id, None) {
            Ok(forked) => {
                let messages = state
                    .store
                    .load_messages(&forked.id)
                    .map_err(|err| TuiError::State(err.to_string()))
                    .unwrap_or_else(|err| {
                        state.status = Some(err.to_string());
                        Vec::new()
                    });
                chat.session = forked;
                chat.messages = messages;
                chat.scroll = 0;
                chat.composer.clear();
                chat.prompt_history = build_prompt_history(&chat.messages);
                chat.history_cursor = None;
                chat.history_draft.clear();
                chat.focus = ChatFocus::Composer;
                chat.activity.clear();
                chat.activity_selected = 0;
                chat.details_open = false;
                chat.tool_details = false;
                chat.running = None;
                state.status = None;
            }
            Err(err) => {
                state.status = Some(format!("failed to fork session: {err}"));
            }
        }
        return ChatNav::Stay;
    }

    if chat.details_open {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                chat.details_open = false;
            }
            _ => {}
        }
        return ChatNav::Stay;
    }

    if chat.focus == ChatFocus::Composer && key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Up => {
                history_prev(chat);
                return ChatNav::Stay;
            }
            KeyCode::Down => {
                history_next(chat);
                return ChatNav::Stay;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Tab => {
            chat.focus = match chat.focus {
                ChatFocus::Composer => ChatFocus::Transcript,
                ChatFocus::Transcript => ChatFocus::Activity,
                ChatFocus::Activity => ChatFocus::Composer,
            };
        }
        KeyCode::Char('t') => {
            chat.tool_details = !chat.tool_details;
        }
        KeyCode::Esc => {
            if !chat.composer.is_empty() {
                chat.composer.clear();
            } else {
                return ChatNav::ToSessions;
            }
        }
        KeyCode::Char('q') => {
            return ChatNav::ToSessions;
        }
        KeyCode::Char('r') => {
            refresh_chat_messages(state, chat);
        }
        KeyCode::Down => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.activity_selected =
                        (chat.activity_selected + 1).min(chat.activity.len().saturating_sub(1));
                }
            } else {
                chat.scroll = chat.scroll.saturating_add(1);
            }
        }
        KeyCode::Up => {
            if chat.focus == ChatFocus::Activity {
                chat.activity_selected = chat.activity_selected.saturating_sub(1);
            } else {
                chat.scroll = chat.scroll.saturating_sub(1);
            }
        }
        KeyCode::Backspace => {
            if chat.focus == ChatFocus::Composer {
                chat.composer.pop();
            }
        }
        KeyCode::Enter => {
            if chat.focus == ChatFocus::Activity {
                if !chat.activity.is_empty() {
                    chat.details_open = true;
                }
                return ChatNav::Stay;
            }

            if chat.focus == ChatFocus::Composer && key.modifiers.contains(KeyModifiers::ALT) {
                chat.composer.push('\n');
                return ChatNav::Stay;
            }

            if chat.running.is_some() {
                return ChatNav::Stay;
            }
            let prompt = chat.composer.trim().to_string();
            if prompt.is_empty() {
                return ChatNav::Stay;
            }
            chat.composer.clear();
            submit_prompt(state, chat, prompt);
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                if chat.focus == ChatFocus::Composer {
                    chat.composer.push(ch);
                }
            }
        }
        _ => {}
    }

    ChatNav::Stay
}

fn refresh_chat_messages(state: &mut AppState, chat: &mut ChatState) {
    match state.store.load_messages(&chat.session.id) {
        Ok(messages) => chat.messages = messages,
        Err(err) => state.status = Some(format!("failed to load transcript: {err}")),
    }
}

fn submit_prompt(state: &mut AppState, chat: &mut ChatState, prompt: String) {
    let Some(config) = state.config.clone() else {
        state.status = Some("config not loaded; cannot run agent".to_string());
        return;
    };
    let Some(executor) = state.executor.clone() else {
        state.status = Some("engine not available; cannot run agent".to_string());
        return;
    };

    let history = match state.store.load_messages(&chat.session.id) {
        Ok(history) => history,
        Err(err) => {
            state.status = Some(format!("failed to load history: {err}"));
            return;
        }
    };

    let cancellation = CancellationToken::new();
    chat.running = Some(RunningCommand {
        cancellation: cancellation.clone(),
    });
    state.status = None;

    let trimmed = prompt.trim();
    if !trimmed.is_empty() {
        if !chat
            .prompt_history
            .last()
            .is_some_and(|last| last.as_str() == trimmed)
        {
            chat.prompt_history.push(trimmed.to_string());
            while chat.prompt_history.len() > 200 {
                chat.prompt_history.remove(0);
            }
        }
    }
    chat.history_cursor = None;
    chat.history_draft.clear();

    chat.focus = ChatFocus::Transcript;

    state.request_seq = state.request_seq.saturating_add(1);
    let seq = state.request_seq;
    let session_id = chat.session.id.clone();
    let request_id = format!("tui-{seq}");

    let tx = state.tx.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(TuiPublisher::new(tx.clone()));
    let runtime = state.runtime.clone();
    runtime.spawn(async move {
        let context = CommandContext::with_cancellation(
            config,
            SessionMeta {
                session_id,
                request_id,
                started_at: SystemTime::now(),
            },
            cancellation,
        );

        let options = AgentOptions {
            allow_write: true,
            allow_edit: true,
            allow_exec: true,
            ..AgentOptions::default()
        };
        let command = Command::Agent {
            prompt,
            options,
            history,
        };

        let result = executor.execute(command, context, publisher).await;
        let (ok, message) = match result {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        };
        let _ = tx.send(InteractiveMsg::RunEnded { ok, message });
    });
}

fn history_prev(chat: &mut ChatState) {
    if chat.prompt_history.is_empty() {
        return;
    }
    let next = match chat.history_cursor {
        None => {
            chat.history_draft = chat.composer.clone();
            chat.prompt_history.len().saturating_sub(1)
        }
        Some(0) => 0,
        Some(idx) => idx.saturating_sub(1),
    };
    chat.history_cursor = Some(next);
    if let Some(value) = chat.prompt_history.get(next) {
        chat.composer = value.clone();
    }
}

fn history_next(chat: &mut ChatState) {
    let Some(idx) = chat.history_cursor else {
        return;
    };
    if idx + 1 >= chat.prompt_history.len() {
        chat.history_cursor = None;
        chat.composer = chat.history_draft.clone();
        return;
    }
    let next = idx + 1;
    chat.history_cursor = Some(next);
    if let Some(value) = chat.prompt_history.get(next) {
        chat.composer = value.clone();
    }
}

fn render(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    match &state.screen {
        Screen::Sessions => render_sessions(frame, state),
        Screen::Chat(chat) => render_chat(frame, state, chat),
    }

    if let Some(pending) = &state.pending_approval {
        render_approval_modal(frame, pending);
    }
}

fn render_sessions(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(frame.area());

    let items = if state.sessions_view.is_empty() {
        if state.sessions_filter.trim().is_empty() {
            vec![ListItem::new("(no sessions)")]
        } else {
            vec![ListItem::new("(no matches)")]
        }
    } else {
        state
            .sessions_view
            .iter()
            .filter_map(|idx| state.sessions.get(*idx))
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
    if !state.sessions_view.is_empty() {
        let idx = state
            .selected
            .min(state.sessions_view.len().saturating_sub(1));
        list_state.select(Some(idx));
    }
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let help = Paragraph::new(Line::from(vec![
        Span::raw("Up/Down: select  "),
        Span::raw("Enter: open  "),
        Span::raw("/: filter  "),
        Span::raw("n: new  "),
        Span::raw("f: fork  "),
        Span::raw("r: refresh  "),
        Span::raw("q: quit"),
        Span::raw("  "),
        Span::styled(
            state.status.as_deref().unwrap_or(""),
            Style::default().fg(Color::Red),
        ),
        Span::raw("  "),
        Span::styled(
            format!("filter={}", if state.sessions_filter.is_empty() { "-" } else { &state.sessions_filter }),
            Style::default().fg(Color::Gray),
        ),
    ]))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(help, chunks[1]);
}

fn render_chat(frame: &mut ratatui::Frame<'_>, app: &AppState, chat: &ChatState) {
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(frame.area());

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(5), Constraint::Length(2)])
        .split(root[0]);

    let title = chat
        .session
        .title
        .as_deref()
        .unwrap_or(chat.session.id.as_str())
        .to_string();

    let mut lines = Vec::new();
    for msg in &chat.messages {
        append_message_lines(&mut lines, msg, chat.tool_details);
    }

    let transcript_border = if chat.focus == ChatFocus::Transcript {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let transcript = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(transcript_border),
        )
        .wrap(Wrap { trim: false })
        .scroll((chat.scroll, 0));
    frame.render_widget(transcript, left[0]);

    let composer_title = if chat.running.is_some() {
        "Prompt (running)"
    } else {
        "Prompt"
    };
    let composer_border = if chat.focus == ChatFocus::Composer {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let composer = Paragraph::new(chat.composer.as_str())
        .block(
            Block::default()
                .title(composer_title)
                .borders(Borders::ALL)
                .border_style(composer_border),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(composer, left[1]);

    let focus_label = match chat.focus {
        ChatFocus::Composer => "focus: composer",
        ChatFocus::Transcript => "focus: transcript",
        ChatFocus::Activity => "focus: activity",
    };

    let tools_label = if chat.tool_details {
        "tools: details"
    } else {
        "tools: summary"
    };

    let help = Paragraph::new(Line::from(vec![
        Span::raw("Tab: focus  "),
        Span::raw(focus_label),
        Span::raw("  "),
        Span::raw("Enter: submit/open  "),
        Span::raw("Alt+Enter: newline  "),
        Span::raw("Alt+Up/Down: history  "),
        Span::raw("Up/Down: scroll/select  "),
        Span::raw("t: tool details  "),
        Span::raw(tools_label),
        Span::raw("  "),
        Span::raw("Ctrl+C: cancel  "),
        Span::raw("Ctrl+N: new  "),
        Span::raw("Ctrl+F: fork  "),
        Span::raw("r: refresh  "),
        Span::raw("Esc: clear/back  "),
        Span::raw("q: sessions"),
        Span::raw("  "),
        Span::styled(
            app.status.as_deref().unwrap_or(""),
            Style::default().fg(Color::Red),
        ),
    ]))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(help, left[2]);

    render_activity(frame, root[1], chat);

    if chat.details_open {
        render_activity_details_modal(frame, chat);
    }
}

fn append_message_lines(lines: &mut Vec<Line<'static>>, msg: &StoredMessage, tool_details: bool) {
    let role = match msg.role {
        MessageRole::System => "System",
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
        MessageRole::Tool => "Tool",
    };

    lines.push(Line::from(vec![Span::styled(
        role,
        Style::default().add_modifier(Modifier::BOLD),
    )]));

    match msg.role {
        MessageRole::Tool => {
            append_tool_message_lines(lines, msg, tool_details);
        }
        _ => {
            append_value_lines(lines, &msg.content, "", 200);

            if !msg.tool_calls.is_empty() {
                lines.push(Line::raw(""));
                for call in &msg.tool_calls {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("Tool: {}", call.name),
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("  id={} ", call.id)),
                    ]));

                    if tool_details {
                        let pretty = serde_json::from_str::<serde_json::Value>(&call.arguments)
                            .ok()
                            .and_then(|value| serde_json::to_string_pretty(&value).ok())
                            .unwrap_or_else(|| call.arguments.clone());
                        append_value_lines(lines, &serde_json::Value::String(pretty), "  ", 64);
                    }
                }
            }
        }
    }

    lines.push(Line::raw(""));
}

fn append_tool_message_lines(lines: &mut Vec<Line<'static>>, msg: &StoredMessage, tool_details: bool) {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    let call_id = msg.tool_call_id.as_deref().unwrap_or("-");

    let content_str = msg.content.as_str().unwrap_or("");
    if let Some((ok, truncated, output)) = parse_tool_payload(content_str) {
        lines.push(Line::raw(format!(
            "Tool: {name}  id={call_id}  ok={ok}  truncated={truncated}"
        )));
        if tool_details {
            lines.push(Line::raw(""));
            lines.push(Line::raw("Output:"));
            append_value_lines(lines, &serde_json::Value::String(output), "  ", 200);
        } else {
            let snippet = output
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !snippet.is_empty() {
                lines.push(Line::raw(format!("  {snippet}")));
            }
        }
        return;
    }

    // Fallback: show raw content.
    if !content_str.is_empty() {
        lines.push(Line::raw(format!("Tool: {name}  id={call_id}")));
        append_value_lines(lines, &msg.content, "  ", 200);
    } else {
        lines.push(Line::raw(format!("Tool: {name}  id={call_id}")));
    }
}

fn parse_tool_payload(content: &str) -> Option<(bool, bool, String)> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let ok = value.get("ok")?.as_bool()?;
    let truncated = value.get("truncated")?.as_bool().unwrap_or(false);
    let output = value
        .get("output")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    Some((ok, truncated, output))
}

fn append_value_lines(lines: &mut Vec<Line<'static>>, value: &serde_json::Value, prefix: &str, max_lines: usize) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::String(text) => {
            append_text_lines(lines, text.as_str(), prefix, max_lines);
        }
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string());
            append_text_lines(lines, pretty.as_str(), prefix, max_lines);
        }
    }
}

fn append_text_lines(lines: &mut Vec<Line<'static>>, text: &str, prefix: &str, max_lines: usize) {
    if text.is_empty() {
        return;
    }
    for (idx, line) in text.lines().enumerate() {
        if idx >= max_lines {
            lines.push(Line::raw(format!("{prefix}...[truncated]...")));
            break;
        }
        if prefix.is_empty() {
            lines.push(Line::raw(line.to_string()));
        } else {
            lines.push(Line::raw(format!("{prefix}{line}")));
        }
    }
}

fn render_activity(frame: &mut ratatui::Frame<'_>, area: Rect, chat: &ChatState) {
    let border = if chat.focus == ChatFocus::Activity {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    if chat.activity.is_empty() {
        let list = List::new(vec![ListItem::new("(no activity)")]).block(
            Block::default()
                .title("Activity")
                .borders(Borders::ALL)
                .border_style(border),
        );
        frame.render_widget(list, area);
        return;
    }

    let available = area.height.saturating_sub(2).max(1) as usize;
    let selected = chat.activity_selected.min(chat.activity.len().saturating_sub(1));
    let half = available / 2;
    let start = if selected > half {
        selected - half
    } else {
        0
    };
    let end = (start + available).min(chat.activity.len());

    let items = chat.activity[start..end]
        .iter()
        .map(|item| ListItem::new(item.summary()))
        .collect::<Vec<_>>();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(selected.saturating_sub(start)));
    let list = List::new(items)
        .block(
            Block::default()
                .title("Activity")
                .borders(Borders::ALL)
                .border_style(border),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_activity_details_modal(frame: &mut ratatui::Frame<'_>, chat: &ChatState) {
    let Some(item) = chat.activity.get(chat.activity_selected) else {
        return;
    };
    let area = centered_rect(90, 80, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Details")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let mut lines = item.details_lines();
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("Enter/Esc: close"),
        Span::raw("  "),
        Span::raw("Tab: focus"),
    ]));

    let modal = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}

fn render_approval_modal(frame: &mut ratatui::Frame<'_>, pending: &PendingApproval) {
    let area = centered_rect(80, 60, frame.area());
    frame.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "Tool approval required",
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!("tool: {}", pending.request.tool)));
    lines.push(Line::raw(format!("permission: {}", pending.request.permission)));
    lines.push(Line::raw(format!("target: {}", pending.request.pattern)));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!("reason: {}", pending.request.reason)));
    lines.push(Line::raw(""));
    lines.push(Line::raw("arguments:"));
    let args = serde_json::to_string_pretty(&pending.request.arguments)
        .unwrap_or_else(|_| "<unprintable>".to_string());
    for line in args.lines().take(12) {
        lines.push(Line::raw(line.to_string()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("a: allow  "),
        Span::raw("d: deny  "),
        Span::raw("Esc: deny"),
    ]));

    let block = Block::default()
        .title("Approval")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let modal = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
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
    use serde_json::Value;

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
            sessions_view: vec![0],
            selected: 0,
            sessions_filter: String::new(),
            sessions_filter_active: false,
            screen: Screen::Sessions,
            status: None,
            defaults: InteractiveDefaults {
                workspace_root: std::path::PathBuf::from("/tmp"),
                model: "null".to_string(),
            },

            pending_approval: None,
            store: SessionStore::with_root(std::path::PathBuf::from("/tmp")),
            config: None,
            executor: None,
            runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
            tx: tokio::sync::mpsc::unbounded_channel().0,
            rx: tokio::sync::mpsc::unbounded_channel().1,
            request_seq: 0,
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
                    "{\"ok\":true,\"truncated\":false,\"output\":\"tool output\"}"
                        .to_string(),
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
                composer: String::new(),
                prompt_history: Vec::new(),
                history_cursor: None,
                history_draft: String::new(),
                focus: ChatFocus::Composer,
                activity: Vec::new(),
                activity_selected: 0,
                details_open: false,
                tool_details: false,
                running: None,
            }),
            status: None,
            defaults: InteractiveDefaults {
                workspace_root: std::path::PathBuf::from("/tmp"),
                model: "null".to_string(),
            },

            pending_approval: None,
            store: SessionStore::with_root(std::path::PathBuf::from("/tmp")),
            config: None,
            executor: None,
            runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
            tx: tokio::sync::mpsc::unbounded_channel().0,
            rx: tokio::sync::mpsc::unbounded_channel().1,
            request_seq: 0,
        };

        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Tool: read"), "text={text}");
        assert!(text.contains("ok=true"), "text={text}");
        assert!(text.contains("tool output"), "text={text}");
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
            defaults: InteractiveDefaults {
                workspace_root: std::path::PathBuf::from("/tmp"),
                model: "null".to_string(),
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
            store: SessionStore::with_root(std::path::PathBuf::from("/tmp")),
            config: None,
            executor: None,
            runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
            tx: tokio::sync::mpsc::unbounded_channel().0,
            rx: tokio::sync::mpsc::unbounded_channel().1,
            request_seq: 0,
        };

        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Tool approval required"), "text={text}");
        assert!(text.contains("tool: read"), "text={text}");
        assert!(text.contains("target: README.md"), "text={text}");
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
                composer: String::new(),
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
                running: None,
            }),
            status: None,
            defaults: InteractiveDefaults {
                workspace_root: std::path::PathBuf::from("/tmp"),
                model: "null".to_string(),
            },

            pending_approval: None,
            store: SessionStore::with_root(std::path::PathBuf::from("/tmp")),
            config: None,
            executor: None,
            runtime: tokio::runtime::Runtime::new().unwrap().handle().clone(),
            tx: tokio::sync::mpsc::unbounded_channel().0,
            rx: tokio::sync::mpsc::unbounded_channel().1,
            request_seq: 0,
        };

        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Details"), "text={text}");
        assert!(text.contains("arguments:"), "text={text}");
        assert!(text.contains("README.md"), "text={text}");
    }
}
