use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};

use super::{
    build_prompt_history, composer_insert_str, compute_sessions_view, disable_raw_mode,
    drain_toasts, enable_raw_mode, event, execute, handle_key, io, render, sort_sessions, AppState,
    CEvent, ChatFocus, ChatState, CrosstermBackend, Duration, EnterAlternateScreen,
    InteractiveServices, InteractiveStart, KeyEventKind, LeaveAlternateScreen, MouseEventKind,
    Screen, Terminal, TuiError,
};

mod events;
mod git;
mod submit;

use self::events::{drain_messages, poll_provider_oauth};
use self::git::refresh_git_stat;
pub(super) use self::submit::submit_prompt;

pub(super) fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    let InteractiveServices {
        backend: session_backend,
        defaults,
        initial_status,
        start,
        runtime,
        handles,
        config,
        executor,
        llm_cell,
        submit_mode,
    } = services;

    // Install a terminal-restoring panic hook so the panic message is legible.
    // Without this, the message prints while the terminal is still in raw mode.
    // TerminalCleanup's Drop still runs during unwinding, so double-restore is harmless.
    let prev_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        prev_panic_hook(info);
    }));

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )
    .map_err(|err| TuiError::Io(err.to_string()))?;
    let _cleanup = TerminalCleanup;

    let term_backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(term_backend).map_err(|err| TuiError::Io(err.to_string()))?;
    terminal
        .clear()
        .map_err(|err| TuiError::Io(err.to_string()))?;

    let mut sessions = session_backend.list_sessions().map_err(TuiError::State)?;
    sort_sessions(&mut sessions);
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
            if let Some(idx) = sessions
                .iter()
                .position(|candidate| candidate.id == session.id)
            {
                selected = idx;
            }
            let messages = session_backend
                .load_messages(&session.id)
                .map_err(TuiError::State)?;
            let prompt_history = build_prompt_history(&messages);
            let initial_composer = if should_submit {
                String::new()
            } else {
                prompt.clone().unwrap_or_default()
            };
            let tokens_in = session.total_input_tokens;
            let tokens_out = session.total_output_tokens;
            let cost = session.cost_usd;
            screen = Screen::Chat(ChatState {
                session,
                messages,
                scroll: 0,
                live_assistant: String::new(),
                live_reasoning: String::new(),
                show_reasoning: false,
                composer_cursor: initial_composer.len(),
                paste_buffer: None,
                composer: initial_composer,
                prompt_history,
                history_cursor: None,
                history_draft: String::new(),
                focus: ChatFocus::Composer,
                activity: Vec::new(),
                activity_selected: 0,
                details_open: false,
                activity_hidden: false,
                tool_details: false,
                output_details: false,
                find: None,
                running: None,
                pending_prompt: None,
                committed_approvals: Vec::new(),
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
        toasts: Vec::new(),
        help_open: false,
        modal: None,
        defaults,
        pending_approval: None,
        approval_selection: 0,
        submit_mode,
        backend: session_backend,
        config,
        executor,
        runtime,
        tx: handles.tx,
        rx: handles.rx,
        request_seq: 0,
        last_area: terminal
            .size()
            .map_err(|err| TuiError::Io(err.to_string()))?,
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell,
        git_stat: None,
    };
    refresh_git_stat(&mut state);

    if let Some(prompt) = auto_submit {
        if !prompt.trim().is_empty() {
            start_auto_submit(&mut state, prompt);
        }
    }

    let mut last_screen_is_chat = matches!(state.screen, Screen::Chat(_));

    loop {
        // Clear terminal when transitioning between Sessions and Chat screens
        // to prevent ghost cells from the previous layout.
        let screen_is_chat = matches!(state.screen, Screen::Chat(_));
        if screen_is_chat != last_screen_is_chat {
            let _ = terminal.clear();
        }
        last_screen_is_chat = screen_is_chat;

        if let Ok(size) = terminal.size() {
            state.last_area = size;
        }
        drain_toasts(&mut state);
        drain_messages(&mut state);
        poll_provider_oauth(&mut state);

        terminal
            .draw(|frame| render(frame, &state))
            .map_err(|err| TuiError::Io(err.to_string()))?;

        if event::poll(Duration::from_millis(50)).map_err(|err| TuiError::Io(err.to_string()))? {
            let evt = event::read().map_err(|err| TuiError::Io(err.to_string()))?;
            match evt {
                CEvent::Key(key) => {
                    if key.kind == KeyEventKind::Press && handle_key(&mut state, key) {
                        return Ok(());
                    }
                }
                CEvent::Paste(text) => handle_paste(&mut state, &text),
                CEvent::Mouse(mouse) => {
                    if let Screen::Chat(mut chat) =
                        std::mem::replace(&mut state.screen, Screen::Sessions)
                    {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                let max = chat.last_max_scroll.get();
                                if max == 0 {
                                    chat.scroll = 0;
                                } else {
                                    chat.scroll = chat.scroll.min(max).saturating_add(3).min(max);
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                // Clamp to last known max_scroll before subtracting so
                                // that over-scrolling up doesn't require many ticks to
                                // "drain" before visual movement resumes.
                                let max = chat.last_max_scroll.get();
                                let clamped = if max > 0 {
                                    chat.scroll.min(max)
                                } else {
                                    chat.scroll
                                };
                                chat.scroll = clamped.saturating_sub(3);
                            }
                            MouseEventKind::Down(_) => {
                                // Click in the activity panel (right 28%) → switch focus there;
                                // click elsewhere → return focus to composer.
                                if !chat.activity_hidden {
                                    let activity_x = state.last_area.width * 72 / 100;
                                    if mouse.column >= activity_x {
                                        chat.focus = ChatFocus::Activity;
                                    } else {
                                        chat.focus = ChatFocus::Composer;
                                    }
                                }
                            }
                            _ => {}
                        }
                        state.screen = Screen::Chat(chat);
                    }
                }
                _ => {}
            }
        }
    }
}

pub(super) fn handle_paste(state: &mut AppState, text: &str) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    if chat.focus == ChatFocus::Composer {
        const LARGE_PASTE_THRESHOLD: usize = 30; // words
        let word_count = normalized.split_whitespace().count();
        if word_count >= LARGE_PASTE_THRESHOLD {
            // Build a short summary: "first two words +N words pasted"
            let mut words = normalized.split_whitespace();
            let w1 = words.next().unwrap_or("");
            let w2 = words.next().unwrap_or("");
            let preview = if w2.is_empty() {
                w1.to_string()
            } else {
                format!("{w1} {w2}")
            };
            let shown = if w2.is_empty() { 1usize } else { 2usize };
            let remaining = word_count.saturating_sub(shown);
            let summary = format!("[{preview} +{remaining} words pasted]");

            // Split composer at cursor so existing text is preserved.
            let before = chat.composer[..chat.composer_cursor].to_string();
            let after = chat.composer[chat.composer_cursor..].to_string();

            // Full text that will be submitted: prefix + paste + suffix.
            let full_text = format!("{before}{normalized}{after}");

            // Displayed text: prefix + summary + suffix.
            let new_cursor = before.len() + summary.len();
            chat.composer = format!("{before}{summary}{after}");
            chat.composer_cursor = new_cursor;
            chat.paste_buffer = Some(full_text);
        } else {
            chat.paste_buffer = None;
            composer_insert_str(&mut chat, &normalized);
        }
    }
    state.screen = Screen::Chat(chat);
}

pub(super) fn start_auto_submit(state: &mut AppState, prompt: String) {
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    submit_prompt(state, &mut chat, prompt);
    state.screen = Screen::Chat(chat);
}

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}
