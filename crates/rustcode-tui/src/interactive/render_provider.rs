use super::{
    Block, Borders, Clear, Color, ConnectMethod, Constraint, Direction, Layout, Line, List,
    ListItem, Modifier, Paragraph, ProviderManagerStep, Rect, Span, Style, Wrap,
};

/// Render the provider manager modal for a given step.
pub(super) fn render_provider_manager_modal(
    frame: &mut ratatui::Frame<'_>,
    step: &ProviderManagerStep,
) {
    let area = super::centered_rect(80, 80, frame.area());
    frame.render_widget(Clear, area);

    match step {
        ProviderManagerStep::List {
            entries,
            query,
            view,
            selected,
        } => {
            let block = Block::default()
                .title("Providers  (Enter: connect  Esc: close  type to search)")
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

            // Search input
            let input_line = Paragraph::new(Line::from(vec![
                Span::styled(
                    "> ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(query.clone()),
            ]))
            .block(
                Block::default()
                    .title("Search providers")
                    .borders(Borders::ALL),
            );
            frame.render_widget(input_line, rows[0]);

            // Provider list
            let total = entries.len();
            let shown = view.len();
            let list_title = format!("Providers ({shown}/{total})");

            let list_items = if view.is_empty() {
                vec![ListItem::new("(no matches)")]
            } else {
                view.iter()
                    .filter_map(|idx| entries.get(*idx))
                    .map(|entry| {
                        let status = if entry.connected { "[+]" } else { "[ ]" };
                        let status_style = if entry.connected {
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(format!("{status} "), status_style),
                            Span::styled(
                                entry.display_name.clone(),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("  ({})", entry.provider_id),
                                Style::default().add_modifier(Modifier::DIM),
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
                Span::raw("[+] = connected  "),
                Span::raw("Enter: manage  "),
                Span::raw("Esc: close  "),
                Span::raw("type to search"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[2]);

            // Place cursor in search box
            let x = rows[0]
                .x
                .saturating_add(4)
                .saturating_add(query.chars().count() as u16);
            let y = rows[0].y.saturating_add(1);
            if x < area.x + area.width && y < area.y + area.height {
                frame.set_cursor_position((x, y));
            }
        }

        ProviderManagerStep::MethodSelect {
            display_name,
            methods,
            selected,
            ..
        } => {
            let block = Block::default()
                .title(format!("Connect: {display_name}"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(inner);

            let list_items = methods
                .iter()
                .enumerate()
                .map(|(i, method)| {
                    let (label, detail) = method_label_detail(*method);
                    let row_style = if i == *selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("  {label:<22}"),
                            row_style.add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(detail.to_string(), row_style.add_modifier(Modifier::DIM)),
                    ]))
                })
                .collect::<Vec<_>>();

            let list = List::new(list_items);
            frame.render_widget(list, rows[0]);

            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Enter: select  "),
                Span::raw("Esc: back  "),
                Span::raw("Up/Down: choose"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[1]);
        }

        ProviderManagerStep::ApiKeyInput {
            display_name,
            env_hint,
            input,
            cursor,
            ..
        } => {
            let title = format!("API key — {display_name}");
            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green));
            let inner = block.inner(area);
            frame.render_widget(block, area);

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(3),
                    Constraint::Length(2),
                ])
                .split(inner);

            // Hint text
            let hint_text = if let Some(env) = env_hint {
                format!("Paste your API key below, or export {env} in your shell.")
            } else {
                "Paste your API key below.".to_string()
            };
            let body = Paragraph::new(vec![Line::raw(hint_text), Line::raw("")])
                .wrap(Wrap { trim: false });
            frame.render_widget(body, rows[0]);

            // Key input field
            let masked: String = input.chars().map(|_| '*').collect();
            let key_input = Paragraph::new(Line::from(vec![Span::raw(masked)])).block(
                Block::default()
                    .title("API Key")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            );
            let key_inner = Block::default()
                .title("API Key")
                .borders(Borders::ALL)
                .inner(rows[1]);
            frame.render_widget(key_input, rows[1]);

            let footer = Paragraph::new(Line::from(vec![
                Span::raw("Enter: save  "),
                Span::raw("Esc: back  "),
                Span::raw("Backspace: delete"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(footer, rows[2]);

            // Cursor position in key input field
            let cx = key_inner.x.saturating_add(*cursor as u16);
            let cy = key_inner.y;
            if cx < area.x + area.width && cy < area.y + area.height {
                frame.set_cursor_position((cx, cy));
            }
        }

        ProviderManagerStep::OAuthStarting {
            provider_id,
            display_name,
        } => {
            render_oauth_waiting(
                frame,
                area,
                display_name,
                provider_id,
                "Starting OAuth flow…",
                None,
            );
        }

        ProviderManagerStep::OAuthPending {
            provider_id,
            display_name,
            verification_uri,
            user_code,
        } => {
            render_oauth_waiting(
                frame,
                area,
                display_name,
                provider_id,
                "Waiting for browser authorization…",
                Some((verification_uri, user_code)),
            );
        }
    }
}

fn method_label_detail(method: ConnectMethod) -> (&'static str, &'static str) {
    match method {
        ConnectMethod::ApiKey => ("API Key", "paste a key directly"),
        ConnectMethod::OAuthDeviceCode => {
            ("OAuth Device Code", "browser auth — no redirect needed")
        }
        ConnectMethod::Disconnect => ("Disconnect", "remove stored credentials"),
    }
}

fn render_oauth_waiting(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    display_name: &str,
    provider_id: &str,
    status_text: &str,
    device_code: Option<(&str, &str)>,
) {
    let block = Block::default()
        .title(format!("OAuth — {display_name} ({provider_id})"))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(2)])
        .split(inner);

    let mut lines = vec![Line::raw(status_text), Line::raw("")];

    if let Some((uri, code)) = device_code {
        lines.push(Line::from(vec![
            Span::styled("URL:  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(uri.to_string()),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("Code: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                code.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::raw(
            "Open the URL above and enter the code to authorize.",
        ));
    } else {
        lines.push(Line::raw("Contacting authorization server…"));
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(body, rows[0]);

    let hint = Paragraph::new(Line::from(vec![Span::styled(
        "Esc: cancel",
        Style::default().fg(Color::DarkGray),
    )]))
    .block(Block::default().borders(Borders::TOP));
    frame.render_widget(hint, rows[1]);
}
