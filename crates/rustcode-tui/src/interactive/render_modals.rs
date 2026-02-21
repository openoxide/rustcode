use super::{
    render_provider_manager_modal, AppState, Block, Borders, Clear, Color, Constraint, Direction,
    InteractiveSubmitMode, Layout, Line, List, ListItem, Modal, Modifier, Paragraph, Rect, Span,
    Style, Wrap, SLASH_COMMANDS,
};

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
        Modal::FileSearch {
            query,
            entries,
            view,
            selected,
        } => {
            let area = centered_rect(80, 75, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("File search")
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

            let input_line = Paragraph::new(Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(query.clone()),
            ]))
            .block(Block::default().title("Search").borders(Borders::ALL));
            frame.render_widget(input_line, rows[0]);

            let total = entries.len();
            let shown = view.len();
            let list_title = format!("Files ({shown}/{total})");
            let list_items = if view.is_empty() {
                vec![ListItem::new("(no matches)")]
            } else {
                view.iter()
                    .filter_map(|idx| entries.get(*idx))
                    .map(|path| ListItem::new(Span::raw(path.clone())))
                    .collect::<Vec<_>>()
            };
            let list = List::new(list_items)
                .block(Block::default().title(list_title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            let mut list_state = ratatui::widgets::ListState::default();
            if !view.is_empty() {
                list_state.select(Some((*selected).min(view.len().saturating_sub(1))));
            }
            frame.render_stateful_widget(list, rows[1], &mut list_state);

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Enter: insert path  "),
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
        Modal::SkillToggle { skills, selected } => {
            let area = centered_rect(72, 65, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("Skills  (Enter: toggle  Esc: close)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(2)])
                .split(inner);

            let list_items = if skills.is_empty() {
                vec![ListItem::new(
                    "No skills loaded — add .md files to ~/.config/rustcode/skills/",
                )]
            } else {
                skills
                    .iter()
                    .enumerate()
                    .map(|(i, (name, desc, enabled))| {
                        let checkbox = if *enabled { "[x]" } else { "[ ]" };
                        let style = if i == *selected {
                            Style::default().add_modifier(Modifier::REVERSED)
                        } else if *enabled {
                            Style::default()
                        } else {
                            Style::default().add_modifier(Modifier::DIM)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(format!("{checkbox} "), style),
                            Span::styled(name.clone(), style.add_modifier(Modifier::BOLD)),
                            Span::styled(
                                if desc.is_empty() {
                                    String::new()
                                } else {
                                    format!("  — {desc}")
                                },
                                style,
                            ),
                        ]))
                    })
                    .collect::<Vec<_>>()
            };
            let list = List::new(list_items);
            frame.render_widget(list, rows[0]);

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Up/Down: select  "),
                Span::raw("Enter/Space: toggle  "),
                Span::raw("Esc: close"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[1]);
        }
        Modal::Feedback {
            rating,
            comment,
            comment_active,
        } => {
            let area = centered_rect(68, 52, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("Feedback")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(5), // rating block: border(1) + 3 content lines + border(1)
                    Constraint::Length(4),
                    Constraint::Length(2),
                ])
                .split(inner);

            // Rating buttons
            let up_style = match rating {
                Some(true) => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                _ => {
                    if *comment_active {
                        Style::default().add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }
                }
            };
            let down_style = match rating {
                Some(false) => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                _ => {
                    if *comment_active {
                        Style::default().add_modifier(Modifier::DIM)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }
                }
            };
            let rating_body = Paragraph::new(vec![
                Line::raw("How useful was this session?"),
                Line::raw(""),
                Line::from(vec![
                    Span::styled(
                        " [+] thumbs up ",
                        if matches!(rating, Some(true)) {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::REVERSED)
                        } else {
                            up_style
                        },
                    ),
                    Span::raw("   "),
                    Span::styled(
                        " [-] thumbs down ",
                        if matches!(rating, Some(false)) {
                            Style::default()
                                .fg(Color::Red)
                                .add_modifier(Modifier::REVERSED)
                        } else {
                            down_style
                        },
                    ),
                    Span::raw("   "),
                    Span::styled(
                        "u/+ = up   d/- = down",
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ]),
            ])
            .block(
                Block::default()
                    .title("Rating")
                    .borders(Borders::ALL)
                    .border_style(if *comment_active {
                        Style::default()
                    } else {
                        Style::default().fg(Color::Cyan)
                    }),
            );
            frame.render_widget(rating_body, rows[0]);

            // Comment field
            let comment_block = Block::default()
                .title("Comment (optional)")
                .borders(Borders::ALL)
                .border_style(if *comment_active {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                });
            let comment_inner = comment_block.inner(rows[1]);
            let comment_para = Paragraph::new(comment.as_str()).wrap(Wrap { trim: false });
            frame.render_widget(comment_block, rows[1]);
            frame.render_widget(comment_para, comment_inner);

            if *comment_active {
                let cx = comment_inner
                    .x
                    .saturating_add(comment.chars().count() as u16);
                let cy = comment_inner.y;
                if cx < area.x + area.width && cy < area.y + area.height {
                    frame.set_cursor_position((cx, cy));
                }
            }

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Tab: switch field  "),
                Span::raw("Enter: submit  "),
                Span::raw("Esc: cancel"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[2]);
        }
        Modal::ModelSelect {
            entries,
            query,
            view,
            selected,
            current_model,
        } => {
            let area = centered_rect(80, 75, frame.area());
            frame.render_widget(Clear, area);

            let block = Block::default()
                .title("Switch model  (Enter: select  Esc: cancel)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));
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

            let input_line = Paragraph::new(Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(query.clone()),
            ]))
            .block(
                Block::default()
                    .title("Search models")
                    .borders(Borders::ALL),
            );
            frame.render_widget(input_line, rows[0]);

            let shown = view.len();
            let total = entries.len();
            let list_title = format!("Models ({shown}/{total})");

            let list_items = if view.is_empty() {
                vec![ListItem::new("(no matches)")]
            } else {
                view.iter()
                    .filter_map(|idx| entries.get(*idx))
                    .map(|entry| {
                        let is_current = entry == current_model
                            || entry.split('/').nth(1).is_some_and(|m| m == current_model)
                            || current_model
                                .split('/')
                                .nth(1)
                                .is_some_and(|m| m == entry.as_str());

                        let (provider, model_id) =
                            entry.split_once('/').unwrap_or(("", entry.as_str()));

                        let marker = if is_current { "*" } else { " " };
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("[{marker}] "),
                                if is_current {
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD)
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            ),
                            Span::styled(
                                format!("{provider:12} "),
                                Style::default().add_modifier(Modifier::DIM),
                            ),
                            Span::styled(
                                model_id.to_string(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                        ]))
                    })
                    .collect::<Vec<_>>()
            };

            let list = List::new(list_items)
                .block(Block::default().title(list_title).borders(Borders::ALL))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

            let mut list_state = ratatui::widgets::ListState::default();
            if !view.is_empty() {
                list_state.select(Some((*selected).min(view.len().saturating_sub(1))));
            }
            frame.render_stateful_widget(list, rows[1], &mut list_state);

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Enter: use model  "),
                Span::raw("Esc: cancel  "),
                Span::raw("Up/Down: select  "),
                Span::raw("type to search"),
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
        Modal::ProviderManager { step } => {
            render_provider_manager_modal(frame, step);
        }
        Modal::SlashHelp { query, selected } => {
            render_slash_help(frame, query, *selected);
        }
        Modal::ErrorDetail { message } => {
            let area = centered_rect(80, 60, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title("Error details")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(2)])
                .split(inner);

            let body = Paragraph::new(message.clone())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: false });
            frame.render_widget(body, rows[0]);

            let hint = Paragraph::new(Line::from(vec![Span::styled(
                "Esc / Enter: dismiss",
                Style::default().fg(Color::DarkGray),
            )]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[1]);
        }
    }
}

pub(super) fn render_help_modal(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![Span::styled(
            "Help",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
    ];

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
    lines.push(Line::raw(
        "  Up/Down/PgUp/PgDn: select  Home/End: first/last",
    ));
    lines.push(Line::raw("  Enter: open session"));
    lines.push(Line::raw("  /: search/filter sessions"));
    lines.push(Line::raw("  Ctrl+N: new session"));
    lines.push(Line::raw("  Ctrl+E: rename session"));
    lines.push(Line::raw("  Ctrl+D: delete session"));
    lines.push(Line::raw("  Ctrl+R: refresh list"));
    lines.push(Line::raw("  Esc / Q: quit"));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Chat",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw(
        "  Alt+Tab: switch focus (Composer ↔ Activity, only when panel is visible)",
    ));
    lines.push(Line::raw(
        "  Esc: focus back to composer / clear composer / back to sessions",
    ));
    lines.push(Line::raw(
        "  Enter: submit prompt (composer) / open details (activity)",
    ));
    lines.push(Line::raw("  Shift+Enter: insert newline in composer"));
    lines.push(Line::raw("  Alt+Up/Down: browse prompt history"));
    lines.push(Line::raw(
        "  Ctrl+K: kill to end of line  Ctrl+U: kill to start of line",
    ));
    lines.push(Line::raw("  Ctrl+Left/Right: jump word in composer"));
    lines.push(Line::raw(
        "  Up/Down: edit text (composer) / select item (activity)",
    ));
    lines.push(Line::raw("  PgUp/PgDn: scroll transcript"));
    lines.push(Line::raw(
        "  / (any focus): move to composer and insert / for slash commands",
    ));
    lines.push(Line::raw("  Ctrl+C: cancel running agent"));
    lines.push(Line::raw("  Ctrl+N: new session"));
    lines.push(Line::raw("  Ctrl+F: fork current session"));
    lines.push(Line::raw("  Ctrl+R: refresh transcript"));
    lines.push(Line::raw(
        "  Ctrl+T: file search (inserts @path into composer)",
    ));
    lines.push(Line::raw("  Ctrl+Q: go to sessions list"));
    lines.push(Line::raw("  Ctrl+P: open command palette"));
    lines.push(Line::raw("  Ctrl+M: switch model (pick a different LLM)"));
    lines.push(Line::raw("  Ctrl+A: manage providers (connect/disconnect)"));
    lines.push(Line::raw("  Ctrl+S: skill toggle overlay"));
    lines.push(Line::raw("  Ctrl+B: give feedback (thumbs up/down)"));
    lines.push(Line::raw("  Ctrl+W: toggle activity panel (show/hide)"));
    lines.push(Line::raw("  Ctrl+O: expand/collapse tool details"));
    lines.push(Line::raw(""));
    lines.push(Line::raw(
        "  Slash commands (type in composer, press Enter):",
    ));
    lines.push(Line::raw(
        "  /help  /sessions  /new  /fork  /reload  /find  /model  /providers  /clear  /skill  /memory",
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

/// Render the slash-command autocomplete popup.
fn render_slash_help(frame: &mut ratatui::Frame<'_>, query: &str, selected: usize) {
    let filtered: Vec<(&str, &str)> = if query.is_empty() {
        SLASH_COMMANDS.to_vec()
    } else {
        let needle = query.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| {
                let stem = cmd
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                stem.to_ascii_lowercase().contains(&needle)
            })
            .copied()
            .collect()
    };

    let area = slash_popup_rect(frame.area(), filtered.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("/ commands  (Tab: complete  Esc: close)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if filtered.is_empty() {
        let msg =
            Paragraph::new("(no matches)").style(Style::default().add_modifier(Modifier::DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let clamped = selected.min(filtered.len().saturating_sub(1));
    let items: Vec<ListItem<'_>> = filtered
        .iter()
        .enumerate()
        .map(|(i, (cmd, desc))| {
            let style = if i == clamped {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(*cmd, style.fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("  ", style),
                Span::styled(*desc, style.fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(clamped));
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, inner, &mut list_state);
}

/// Compute the popup rect for the slash-help popup.
///
/// Anchored to the lower-left of the terminal, just above the composer+footer
/// (~8 rows from the bottom).  Width is half the terminal; height grows with
/// the number of matches up to 10 rows.
fn slash_popup_rect(r: Rect, item_count: usize) -> Rect {
    let content_h = (item_count as u16).max(1).min(10);
    // +2 for borders
    let h = content_h + 2;
    let w = r.width / 2;
    // Position above the composer+footer (~8 rows from bottom)
    let y = r.y + r.height.saturating_sub(h + 8);
    Rect {
        x: r.x + 1,
        y,
        width: w,
        height: h,
    }
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
