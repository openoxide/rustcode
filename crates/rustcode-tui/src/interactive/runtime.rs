use std::collections::VecDeque;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use futures_util::StreamExt;

use super::inline_terminal::InlineTerminal;
use super::state::compute_desired_height;
use super::{
    composer_insert_str, compute_sessions_view, disable_raw_mode, drain_toasts, enable_raw_mode,
    execute, handle_key, io, render, sort_sessions, AppState, ApprovalMode, ChatFocus, ChatState,
    InteractiveServices, InteractiveStart, Screen, TuiError,
};

mod event_stream;
mod events;
pub(crate) mod frame_scheduler;
mod git;
mod submit;

use self::event_stream::TuiEvent;
use self::events::{drain_messages, poll_provider_oauth};
use self::frame_scheduler::FrameScheduler;
use self::git::refresh_git_stat_async;
pub(super) use self::submit::{submit_compact, submit_prompt};

/// Run the interactive TUI event loop using async `tokio::select!`.
///
/// # Errors
/// Returns `TuiError` on terminal I/O or state failures.
pub(super) async fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    let InteractiveServices {
        backend: session_backend,
        defaults,
        initial_status,
        start,
        handles,
        config,
        executor,
        llm_cell,
        submit_mode,
        mode_flag,
    } = services;

    let keyboard_enhancement_enabled = matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    );

    // Install a terminal-restoring panic hook so the panic message is legible.
    let prev_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        if keyboard_enhancement_enabled {
            let _ = execute!(
                stdout,
                PopKeyboardEnhancementFlags,
                DisableMouseCapture,
                DisableBracketedPaste
            );
        } else {
            let _ = execute!(stdout, DisableMouseCapture, DisableBracketedPaste);
        }
        // Reset scroll region and show cursor for legible panic output.
        let _ = std::io::Write::write_all(&mut stdout, b"\x1b[r\n");
        let _ = crossterm::cursor::Show;
        let _ = std::io::Write::flush(&mut stdout);
        prev_panic_hook(info);
    }));

    let mut stdout = io::stdout();
    enable_raw_mode().map_err(|err| TuiError::Io(err.to_string()))?;
    // Inline mode: no EnterAlternateScreen — previous terminal content stays visible.
    if keyboard_enhancement_enabled {
        execute!(
            stdout,
            EnableMouseCapture,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )
        .map_err(|err| TuiError::Io(err.to_string()))?;
    } else {
        execute!(stdout, EnableMouseCapture, EnableBracketedPaste)
            .map_err(|err| TuiError::Io(err.to_string()))?;
    }

    let mut terminal = InlineTerminal::new()?;
    let _cleanup = TerminalCleanup {
        keyboard_enhancement_enabled,
    };

    // ── Frame scheduler ──
    let (frame_requester, draw_rx, scheduler) = FrameScheduler::new();
    tokio::spawn(scheduler.run());

    // ── TUI event stream (crossterm + draw signals) ──
    let mut tui_events = event_stream::TuiEventStream::new(draw_rx);

    let mut sessions = session_backend.list_sessions().map_err(TuiError::State)?;
    sort_sessions(&mut sessions);
    let sessions_view = compute_sessions_view(&sessions, "");
    let prompt_history_store = rustcode_state::PromptHistoryStore::open_default();
    let global_prompt_history = match prompt_history_store.load() {
        Ok(history) => history,
        Err(err) => {
            eprintln!("warning: failed to load prompt history: {err}");
            Vec::new()
        }
    };
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
            let messages = if crate::is_draft_session_id(&session.id) {
                Vec::new()
            } else {
                session_backend
                    .load_messages(&session.id)
                    .map_err(TuiError::State)?
            };
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
                total_input_tokens: tokens_in,
                total_output_tokens: tokens_out,
                last_total_tokens: tokens_in + tokens_out,
                context_limit: 0,
                cost_usd: cost,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                last_max_scroll: std::cell::Cell::new(0),
                last_transcript_wrapped_count: std::cell::Cell::new(0),
                run_started_at: None,
                last_run_elapsed: None,
                plan_title: None,
                plan_steps: Vec::new(),
                todos: Vec::new(),
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
        prompt_history_store,
        global_prompt_history,
        history_cursor: None,
        history_draft: String::new(),
        pending_approval: None,
        approval_selection: 0,
        approval_mode: ApprovalMode::Normal,
        mode_flag,
        submit_mode,
        backend: session_backend,
        config,
        executor,
        frame_requester: frame_requester.clone(),
        tx: handles.tx,
        rx: handles.rx,
        request_seq: 0,
        last_area: {
            let (w, h) = InlineTerminal::size()?;
            ratatui::layout::Size {
                width: w,
                height: h,
            }
        },
        provider_oauth_start_rx: None,
        provider_oauth_done_rx: None,
        llm_cell,
        git_stat: None,
    };
    refresh_git_stat_async(state.defaults.workspace_root.clone(), state.tx.clone());

    if let Some(prompt) = auto_submit {
        if !prompt.trim().is_empty() {
            start_auto_submit(&mut state, prompt);
        }
    }

    // Request an initial draw so the UI appears immediately.
    frame_requester.schedule_frame();

    let mut _last_screen_is_chat = matches!(state.screen, Screen::Chat(_));

    // Periodic animation tick — ensures spinners, elapsed timers, and thinking
    // animations update smoothly even when no engine messages or input arrive.
    // 100ms (10 Hz) is enough for text-mode spinners/counters.
    let mut animation_tick = tokio::time::interval(std::time::Duration::from_millis(100));
    animation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(msg) = state.rx.recv() => {
                events::process_message(&mut state, msg);
                frame_requester.schedule_frame();
            }
            Some(event) = tui_events.next() => {
                match event {
                    TuiEvent::Draw => {
                        // Drain additional queued engine messages before drawing.
                        drain_messages(&mut state);
                        drain_toasts(&mut state);
                        poll_provider_oauth(&mut state);

                        // Track screen transitions for viewport resize.
                        let screen_is_chat = matches!(state.screen, Screen::Chat(_));
                        _last_screen_is_chat = screen_is_chat;

                        // Update real terminal size.
                        if let Ok((w, h)) = InlineTerminal::size() {
                            state.last_area = ratatui::layout::Size { width: w, height: h };
                        }

                        let desired = compute_desired_height(&state);
                        terminal.draw(desired, |frame| render(frame, &state))?;
                    }
                    TuiEvent::Key(key) => {
                        if handle_key(&mut state, key) {
                            return Ok(());
                        }
                        frame_requester.schedule_frame();
                    }
                    TuiEvent::Paste(text) => {
                        handle_paste(&mut state, &text);
                        frame_requester.schedule_frame();
                    }
                    TuiEvent::Mouse(mouse) => {
                        handle_mouse(&mut state, mouse);
                        frame_requester.schedule_frame();
                    }
                }
            }
            // Periodic tick for animations (spinners, elapsed time, thinking dots).
            _ = animation_tick.tick() => {
                frame_requester.schedule_frame();
            }
        }
    }
}

/// Handle a mouse event in the chat screen.
fn handle_mouse(state: &mut AppState, mouse: crossterm::event::MouseEvent) {
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
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
            let max = chat.last_max_scroll.get();
            let clamped = if max > 0 {
                chat.scroll.min(max)
            } else {
                chat.scroll
            };
            chat.scroll = clamped.saturating_sub(3);
        }
        MouseEventKind::Down(_) => {
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

pub(super) fn handle_paste(state: &mut AppState, text: &str) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let Screen::Chat(mut chat) = std::mem::replace(&mut state.screen, Screen::Sessions) else {
        return;
    };
    if chat.focus == ChatFocus::Composer {
        const LARGE_PASTE_THRESHOLD: usize = 30; // words
        let word_count = normalized.split_whitespace().count();
        if word_count >= LARGE_PASTE_THRESHOLD {
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

            let before = chat.composer[..chat.composer_cursor].to_string();
            let after = chat.composer[chat.composer_cursor..].to_string();

            let full_text = format!("{before}{normalized}{after}");

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

struct TerminalCleanup {
    keyboard_enhancement_enabled: bool,
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        if self.keyboard_enhancement_enabled {
            let _ = execute!(
                stdout,
                PopKeyboardEnhancementFlags,
                DisableMouseCapture,
                DisableBracketedPaste
            );
        } else {
            let _ = execute!(stdout, DisableMouseCapture, DisableBracketedPaste);
        }
        // Reset scroll region, show cursor, and newline for clean shell prompt.
        let _ = execute!(stdout, crossterm::cursor::Show);
        let _ = std::io::Write::write_all(&mut stdout, b"\x1b[r\n");
        let _ = std::io::Write::flush(&mut stdout);
    }
}
