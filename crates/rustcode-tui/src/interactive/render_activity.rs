use super::{
    centered_rect, ActivityItem, Block, Borders, ChatFocus, ChatState, Clear, Color, Duration,
    Line, List, ListItem, Modifier, Paragraph, Rect, Span, Style, SystemTime, Wrap,
};

pub(super) fn render_activity(frame: &mut ratatui::Frame<'_>, area: Rect, chat: &ChatState) {
    let border = if chat.focus == ChatFocus::Activity {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let spinner = if chat.running.is_some() {
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();
        // Heavy-braille circle spinner: ⣾⣽⣻⢿⡿⣟⣯⣷ (8 frames × 80 ms)
        const SPIN: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];
        SPIN[(ms / 80 % 8) as usize]
    } else {
        ' '
    };
    let base_title = if chat.running.is_some() {
        format!("Activity {spinner}")
    } else {
        "Activity".to_string()
    };

    let title_line = Line::raw(base_title);

    if chat.activity.is_empty() {
        let empty = if chat.running.is_some() {
            "(running...)"
        } else {
            "(no activity yet)"
        };
        let list = List::new(vec![ListItem::new(empty)]).block(
            Block::default()
                .title(title_line)
                .borders(Borders::ALL)
                .border_style(border),
        );
        frame.render_widget(list, area);
        return;
    }

    let available = area.height.saturating_sub(2).max(1) as usize;
    let selected = chat
        .activity_selected
        .min(chat.activity.len().saturating_sub(1));
    let half = available / 2;
    let start = selected.saturating_sub(half);
    let end = (start + available).min(chat.activity.len());

    let items = chat.activity[start..end]
        .iter()
        .map(|item| {
            let (tag, style) = match item {
                ActivityItem::CommandAccepted { .. } => ("cmd", Style::default().fg(Color::Cyan)),
                ActivityItem::ToolCall { .. } => ("tool", Style::default().fg(Color::Magenta)),
                ActivityItem::ToolResult { ok, .. } => {
                    if *ok {
                        ("ok", Style::default().fg(Color::Green))
                    } else {
                        ("fail", Style::default().fg(Color::Red))
                    }
                }
                ActivityItem::OutputChunk { .. } => {
                    ("out", Style::default().add_modifier(Modifier::DIM))
                }
                ActivityItem::ReasoningChunk { .. } => {
                    ("think", Style::default().fg(Color::Blue).add_modifier(Modifier::DIM))
                }
                ActivityItem::Warning { .. } => ("warn", Style::default().fg(Color::Magenta)),
                ActivityItem::Failure { .. } => ("error", Style::default().fg(Color::Red)),
                ActivityItem::Completed => ("done", Style::default().fg(Color::Green)),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{tag}] "), style.add_modifier(Modifier::BOLD)),
                Span::raw(item.summary()),
            ]))
        })
        .collect::<Vec<_>>();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(selected.saturating_sub(start)));
    let list = List::new(items)
        .block(
            Block::default()
                .title(title_line)
                .borders(Borders::ALL)
                .border_style(border),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, area, &mut list_state);
}

pub(super) fn render_activity_details_modal(frame: &mut ratatui::Frame<'_>, chat: &ChatState) {
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
    lines.push(Line::raw("Enter/Esc: close"));

    let modal = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}
