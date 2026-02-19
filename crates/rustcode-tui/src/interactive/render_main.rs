use super::{
    apply_find_highlight, build_transcript_lines, composer_cursor_visual, format_age,
    render_activity, render_activity_details_modal, render_approval_modal, render_help_modal,
    render_modal, AppState, Block, Borders, ChatFocus, ChatState, Color, Constraint, Direction,
    Layout, Line, List, ListItem, Modifier, Paragraph, Screen, Span, Style, ToastVariant, Wrap,
};

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

    let focus_label = match chat.focus {
        ChatFocus::Composer => "composer",
        ChatFocus::Transcript => "transcript",
        ChatFocus::Activity => "activity",
    };
    let model_label = app.defaults.model.as_str();
    let running_indicator = if chat.running.is_some() { " ⟳" } else { "" };

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
            "  ^E:details",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    let status_line = Line::from(vec![
        Span::styled(
            format!(" {focus_label}{running_indicator}  [{model_label}]  "),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(status_text, status_style),
        error_hint,
    ]);

    // Line 2: condensed key hints; swap ?:help for ^E:error when error is present
    let hints_text = if has_error {
        " ^P:cmds  ^N:new  ^Q:sessions  ^C:cancel  Tab:focus  Enter:send  /:cmd  ^E:error"
    } else {
        " ^P:cmds  ^N:new  ^Q:sessions  ^C:cancel  Tab:focus  Enter:send  /:cmd  ?:help"
    };
    let hints_line = Line::from(vec![Span::styled(
        hints_text,
        Style::default().fg(Color::DarkGray),
    )]);

    let help = Paragraph::new(vec![status_line, hints_line]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(help, left[2]);

    render_activity(frame, root[1], chat);
    if chat.details_open {
        render_activity_details_modal(frame, chat);
    }
}
