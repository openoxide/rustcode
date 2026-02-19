use super::{
    apply_find_highlight, build_transcript_lines, composer_cursor_visual, format_age,
    render_activity, render_activity_details_modal, render_approval_modal, render_help_modal,
    render_modal, render_settings, AppState, Block, Borders, ChatFocus, ChatState, Color,
    Constraint, Direction, Layout, Line, List, ListItem, Modifier, Paragraph, Screen, Span, Style,
    ToastVariant, Wrap,
};

/// Timeout for typing indicator (milliseconds).
const TYPING_TIMEOUT_MS: u128 = 2000;

pub(super) fn render(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    match &state.screen {
        Screen::Sessions => render_sessions(frame, state),
        Screen::Chat(chat) => render_chat(frame, state, chat),
    }

    if let Some(pending) = &state.pending_approval {
        render_approval_modal(frame, pending);
    }
    if let Some(modal) = &state.modal {
        render_modal(frame, modal);
    }
    if state.help_open {
        render_help_modal(frame, state);
    }
}

pub(super) fn render_sessions(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(frame.area());

    let items = if state.sessions_view.is_empty() {
        if state.sessions_filter.trim().is_empty() {
            vec![ListItem::new("(no sessions yet - press n to create one)")]
        } else {
            vec![ListItem::new("(no matches)")]
        }
    } else {
        state
            .sessions_view
            .iter()
            .filter_map(|idx| state.sessions.get(*idx))
            .map(|session| {
                let age = format_age(session.updated_at_unix_ms);
                let fork_marker = if session.parent_id.is_some() {
                    "⑂ "
                } else {
                    "  "
                };
                // Show title if set; otherwise show "(new session)" for untitled ones
                let title = session.title.as_deref().unwrap_or("(new session)");
                let id_short = &session.id;
                ListItem::new(Line::from(vec![
                    Span::styled(fork_marker, Style::default().fg(Color::Cyan)),
                    Span::styled(
                        title.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        id_short.clone(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::raw("  "),
                    Span::styled(age, Style::default().add_modifier(Modifier::DIM)),
                ]))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("Sessions ({})", state.sessions_view.len()))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ratatui::widgets::ListState::default();
    if !state.sessions_view.is_empty() {
        let idx = state
            .selected
            .min(state.sessions_view.len().saturating_sub(1));
        list_state.select(Some(idx));
    }
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let (status_text, status_style) = if let Some(toast) = state.toasts.last() {
        let style = match toast.variant {
            ToastVariant::Info => Style::default().fg(Color::Cyan),
            ToastVariant::Success => Style::default().fg(Color::Green),
            ToastVariant::Warning => Style::default().fg(Color::Magenta),
            ToastVariant::Error => Style::default().fg(Color::Red),
        };
        (toast.message.as_str(), style)
    } else {
        ("", Style::default())
    };
    let model_label = state.defaults.model.as_str();
    let filter_display = if state.sessions_filter.is_empty() {
        String::new()
    } else {
        format!("  filter:{}", state.sessions_filter)
    };
    let help = Paragraph::new(Line::from(vec![
        Span::styled(
            " ?: help  Ctrl+P: cmds  ↑↓: select  Enter: open  /: filter  ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "Ctrl+N: new  Ctrl+E: rename  Ctrl+D: delete  Ctrl+R: refresh  Esc: quit",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(filter_display, Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("  [{model_label}]"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::raw("  "),
        Span::styled(status_text, status_style),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(help, chunks[1]);
}

pub(super) fn render_chat(frame: &mut ratatui::Frame<'_>, app: &AppState, chat: &ChatState) {
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(72), Constraint::Percentage(28)])
        .split(frame.area());

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(3),
        ])
        .split(root[0]);

    // Show title if renamed, otherwise just show "(new session)"
    let session_title = chat
        .session
        .title
        .as_deref()
        .unwrap_or("(new session)")
        .to_string();
    let mut transcript_title_spans = vec![
        Span::styled(session_title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            chat.session.id.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
    ];
    if let Some(find) = &chat.find {
        if !find.query.trim().is_empty() {
            transcript_title_spans.push(Span::raw("  "));
            transcript_title_spans.push(Span::styled(
                format!(
                    "find: {} ({}/{})",
                    find.query,
                    find.current.saturating_add(1),
                    find.matches.len()
                ),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
    }

    let mut lines = build_transcript_lines(chat);
    lines = apply_find_highlight(lines, chat.find.as_ref());

    let transcript_border = if chat.focus == ChatFocus::Transcript {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let transcript_inner_h = left[0].height.saturating_sub(2).max(1) as usize;
    let max_scroll = lines.len().saturating_sub(transcript_inner_h) as u16;
    let from_bottom = chat.scroll.min(max_scroll);
    let scroll_top = max_scroll.saturating_sub(from_bottom);

    let transcript = Paragraph::new(lines)
        .block(
            Block::default()
                .title(Line::from(transcript_title_spans))
                .borders(Borders::ALL)
                .border_style(transcript_border),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_top, 0));
    frame.render_widget(transcript, left[0]);

    let focus_label = match chat.focus {
        ChatFocus::Composer => "composer",
        ChatFocus::Transcript => "transcript",
        ChatFocus::Activity => "activity",
    };
    let mode_label = match app.submit_mode {
        super::InteractiveSubmitMode::Agent => "agent",
        super::InteractiveSubmitMode::Run => "run",
    };
    let model_label = if chat.session.model.trim().is_empty() {
        app.defaults.model.as_str()
    } else {
        chat.session.model.as_str()
    };

    let is_typing = chat
        .last_typing_time
        .is_some_and(|t| t.elapsed().as_millis() < TYPING_TIMEOUT_MS);
    let prompt_label = if chat.running.is_some() {
        "Prompt (running)"
    } else if is_typing {
        "Prompt (typing...)"
    } else {
        "Prompt"
    };
    let composer_title = Line::from(vec![
        Span::styled(prompt_label, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            format!("[focus:{focus_label}]"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[model:{model_label}]"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[mode:{mode_label}]"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let composer_border = if chat.focus == ChatFocus::Composer {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let composer_text = if chat.composer.is_empty() {
        "Type a prompt... (Enter to submit, Alt+Enter for newline)"
    } else {
        chat.composer.as_str()
    };
    let composer_style = if chat.composer.is_empty() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default()
    };
    let composer_area = left[1];
    let inner_w = composer_area.width.saturating_sub(2);
    let inner_h = composer_area.height.saturating_sub(2);
    let (cursor_row, cursor_col) = if chat.focus == ChatFocus::Composer {
        composer_cursor_visual(&chat.composer, chat.composer_cursor, inner_w)
    } else {
        (0, 0)
    };
    let composer_scroll = if chat.focus == ChatFocus::Composer {
        cursor_row.saturating_sub(inner_h.saturating_sub(1) as usize)
    } else {
        0
    };

    let composer = Paragraph::new(composer_text)
        .style(composer_style)
        .block(
            Block::default()
                .title(composer_title)
                .borders(Borders::ALL)
                .border_style(composer_border),
        )
        .wrap(Wrap { trim: false })
        .scroll((composer_scroll as u16, 0));
    frame.render_widget(composer, composer_area);

    if chat.focus == ChatFocus::Composer
        && !app.help_open
        && app.modal.is_none()
        && app.pending_approval.is_none()
        && !chat.details_open
    {
        let x = composer_area
            .x
            .saturating_add(1)
            .saturating_add(cursor_col as u16);
        let y = composer_area
            .y
            .saturating_add(1)
            .saturating_add((cursor_row.saturating_sub(composer_scroll)) as u16);
        if x < composer_area.x + composer_area.width && y < composer_area.y + composer_area.height {
            frame.set_cursor_position((x, y));
        }
    }

    // Line 1: status/error — truncated to fit, always visible
    let err_max = 55_usize;
    let (status_text, status_style, has_error) = if let Some(ref s) = app.status {
        let truncated = if s.chars().count() > err_max {
            format!("{}…", s.chars().take(err_max).collect::<String>())
        } else {
            s.clone()
        };
        (
            truncated,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            true,
        )
    } else if let Some(toast) = app.toasts.last() {
        let style = match toast.variant {
            ToastVariant::Info => Style::default().fg(Color::Cyan),
            ToastVariant::Success => Style::default().fg(Color::Green),
            ToastVariant::Warning => Style::default().fg(Color::Magenta),
            ToastVariant::Error => Style::default().fg(Color::Red),
        };
        (toast.message.clone(), style, false)
    } else {
        (String::new(), Style::default(), false)
    };
    // Bright yellow hint when error is set — draws attention
    let error_hint = if has_error {
        Span::styled(
            "  Ctrl+E:details",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    let mut status_spans = Vec::new();
    if chat.running.is_some() {
        status_spans.push(Span::styled(
            " running  ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    status_spans.push(Span::styled(status_text, status_style));
    status_spans.push(error_hint);
    let status_line = Line::from(status_spans);

    // When the composer starts with '?' show expanded bindings; otherwise a compact hint.
    let composer_starts_with_query = chat.composer.starts_with('?');
    let hints_line = if composer_starts_with_query {
        // Expanded bindings visible when user types '?' first
        Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                "Ctrl+P",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cmds  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+N",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":new  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+Q",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":sessions  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+C",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cancel  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Alt+Tab",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":focus  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":send  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "/",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            if has_error {
                Span::styled(":cmd  Ctrl+E:error", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(":cmd", Style::default().fg(Color::DarkGray))
            },
        ])
    } else {
        // Compact hint — just enough to orient a new user
        let error_part = if has_error {
            Span::styled(
                "  Ctrl+E:error",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        Line::from(vec![
            Span::styled(" Ctrl+P", Style::default().fg(Color::DarkGray)),
            Span::styled(" cmds", Style::default().fg(Color::DarkGray)),
            Span::styled("   /", Style::default().fg(Color::DarkGray)),
            Span::styled(" cmd", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "   ? bindings",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
            error_part,
        ])
    };

    let help = Paragraph::new(vec![status_line, hints_line]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(help, left[2]);

    // Right panel: split to match left panel structure
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Activity panel (matches transcript + composer)
            Constraint::Length(3), // Settings panel (matches footer)
        ])
        .split(root[1]);

    render_activity(frame, right[0], chat);
    render_settings(frame, right[1], app);
    if chat.details_open {
        render_activity_details_modal(frame, chat);
    }
}
