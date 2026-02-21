use super::{
    centered_rect, Block, Borders, Clear, Color, Constraint, Direction, Layout, Line, List,
    ListItem, Modifier, Paragraph, Span, Style,
};

pub(super) fn render_model_select_modal(
    frame: &mut ratatui::Frame<'_>,
    entries: &[String],
    query: &str,
    view: &[usize],
    selected: usize,
    current_model: &str,
) {
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
        Span::raw(query.to_string()),
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

                let (provider, model_id) = entry.split_once('/').unwrap_or(("", entry.as_str()));
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
        list_state.select(Some(selected.min(view.len().saturating_sub(1))));
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
