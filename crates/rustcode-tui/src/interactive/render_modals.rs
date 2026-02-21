use super::{
    render_provider_manager_modal, AppState, Block, Borders, Clear, Color, Constraint, Direction,
    Layout, Line, List, ListItem, Modal, Modifier, Paragraph, Span, Style, Wrap, SLASH_COMMANDS,
};

mod helpers;
mod model_select;

pub(super) use helpers::centered_rect;
pub(super) use helpers::render_help_modal;

use helpers::render_slash_help;
use model_select::render_model_select_modal;

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
            render_model_select_modal(frame, entries, query, view, *selected, current_model);
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
