use super::*;

pub(super) fn render_modal(frame: &mut ratatui::Frame<'_>, modal: &Modal) {
    match modal {
        Modal::CommandPalette {
            query,
            selected,
            items,
            view,
        } => {
            let area = centered_rect(80, 70, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("Commands")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            let input = Paragraph::new(Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(query.clone()),
            ]))
            .block(Block::default().title("Search").borders(Borders::ALL))
            .wrap(Wrap { trim: false });
            frame.render_widget(input, rows[0]);

            let list_items = if view.is_empty() {
                vec![ListItem::new("(no matches)")]
            } else {
                view.iter()
                    .filter_map(|idx| items.get(*idx))
                    .map(|item| {
                        let style = if item.enabled {
                            Style::default()
                        } else {
                            Style::default().add_modifier(Modifier::DIM)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(item.title.clone(), style.add_modifier(Modifier::BOLD)),
                            Span::raw("  "),
                            Span::styled(item.detail.clone(), style.add_modifier(Modifier::DIM)),
                        ]))
                    })
                    .collect::<Vec<_>>()
            };
            let list = List::new(list_items)
                .block(Block::default().title("Actions").borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            let mut list_state = ratatui::widgets::ListState::default();
            if !view.is_empty() {
                list_state.select(Some((*selected).min(view.len().saturating_sub(1))));
            }
            frame.render_stateful_widget(list, rows[1], &mut list_state);

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Enter: run  "),
                Span::raw("Esc: close  "),
                Span::raw("Up/Down: select"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[2]);

            let x = rows[0]
                .x
                .saturating_add(4)
                .saturating_add(query.chars().count() as u16);
            let y = rows[0].y.saturating_add(1);
            if x < area.x + area.width && y < area.y + area.height {
                frame.set_cursor_position((x, y));
            }
        }
        Modal::Search {
            query,
            current,
            matches,
        } => {
            let area = centered_rect(70, 35, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("Search transcript")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(inner);

            let header = Paragraph::new(Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(query.clone()),
            ]))
            .block(Block::default().title("Find").borders(Borders::ALL));
            frame.render_widget(header, rows[0]);

            let status = if query.trim().is_empty() {
                "Type to search".to_string()
            } else if matches.is_empty() {
                "No matches".to_string()
            } else {
                format!("Match {}/{}", current.saturating_add(1), matches.len())
            };
            let body = Paragraph::new(vec![
                Line::raw(status),
                Line::raw(""),
                Line::raw("Enter/n: next    N: prev"),
                Line::raw("Esc: close"),
            ])
            .wrap(Wrap { trim: false });
            frame.render_widget(body, rows[1]);

            let hint = Paragraph::new(Line::from(vec![Span::raw(
                "Tip: while transcript focused, / opens search; n/N navigate",
            )]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[2]);

            let x = rows[0]
                .x
                .saturating_add(4)
                .saturating_add(query.chars().count() as u16);
            let y = rows[0].y.saturating_add(1);
            if x < area.x + area.width && y < area.y + area.height {
                frame.set_cursor_position((x, y));
            }
        }
        Modal::Rename {
            session_id,
            input,
            cursor,
        } => {
            let area = centered_rect(70, 40, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title("Rename session")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta));
            let inner = block.inner(area);

            let lines = vec![
                Line::raw(format!("id: {session_id}")),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Title: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(input.clone()),
                ]),
                Line::raw(""),
                Line::raw("Enter: save    Esc: cancel"),
            ];

            frame.render_widget(Paragraph::new(lines).block(block), area);

            let title_prefix = "Title: ".chars().count() as u16;
            let x = inner
                .x
                .saturating_add(title_prefix)
                .saturating_add(*cursor as u16);
            let y = inner.y.saturating_add(2);
            if x < area.x + area.width && y < area.y + area.height {
                frame.set_cursor_position((x, y));
            }
        }
        Modal::DeleteConfirm { session_id, title } => {
            let area = centered_rect(70, 30, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title("Delete session")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));
            let lines = vec![
                Line::raw("This will delete the session from disk."),
                Line::raw(""),
                Line::raw(format!("id: {session_id}")),
                Line::raw(format!("title: {title}")),
                Line::raw(""),
                Line::raw("y: delete    n/Esc: cancel"),
            ];
            frame.render_widget(Paragraph::new(lines).block(block), area);
        }
    }
}

pub(super) fn render_help_modal(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        "Help",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Global",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw("  ?: toggle help"));
    lines.push(Line::raw("  Ctrl+P: command palette"));
    lines.push(Line::raw(
        "    - try: session <id>, rename <title>, search <text>",
    ));
    lines.push(Line::raw("  q: quit / back"));
    lines.push(Line::raw("  Esc: close modal / clear / back"));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Sessions",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw("  Up/Down: select"));
    lines.push(Line::raw("  Enter: open"));
    lines.push(Line::raw("  /: filter"));
    lines.push(Line::raw("  n: new"));
    lines.push(Line::raw("  f: fork"));
    lines.push(Line::raw("  e: rename"));
    lines.push(Line::raw("  d: delete"));
    lines.push(Line::raw("  r: refresh"));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Chat",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw("  Tab: focus (composer/transcript/activity)"));
    lines.push(Line::raw("  Enter: submit (composer) / open (activity)"));
    lines.push(Line::raw("  Alt+Enter: newline (composer)"));
    lines.push(Line::raw(
        "  Arrow keys: edit (composer) / scroll (transcript) / select (activity)",
    ));
    lines.push(Line::raw("  / (transcript): search"));
    lines.push(Line::raw("  n/N (transcript): next/prev match"));
    lines.push(Line::raw("  Alt+Up/Down: prompt history"));
    lines.push(Line::raw("  Ctrl+C: cancel running"));
    lines.push(Line::raw("  Ctrl+N: new session"));
    lines.push(Line::raw("  Ctrl+F: fork session"));
    lines.push(Line::raw("  r: refresh transcript"));
    lines.push(Line::raw("  t: toggle tool transcript mode"));
    lines.push(Line::raw(
        "  /commands: /help /sessions /new /fork /reload /tools",
    ));

    if state.submit_mode == InteractiveSubmitMode::Run {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled(
            "Attach mode",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::raw("  This TUI is attached to a remote server."));
        lines.push(Line::raw("  Tool approvals may not be supported."));
    }

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(dialog, area);
}

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
