use super::{
    centered_rect, ActivityItem, Block, Borders, ChatFocus, ChatState, Clear, Duration, Line, List,
    ListItem, Modifier, Paragraph, Rect, Span, Style, SystemTime, Wrap,
};
use crate::interactive::theme;

const ACTIVITY_SPIN: &[char] = &['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

pub(super) fn render_activity(frame: &mut ratatui::Frame<'_>, area: Rect, chat: &ChatState) {
    frame.render_widget(Clear, area);

    let border = if chat.focus == ChatFocus::Activity {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default()
    };

    let spinner = if chat.running.is_some() {
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();
        ACTIVITY_SPIN[(ms / 80 % 8) as usize]
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
    let start = if selected + 1 >= chat.activity.len() {
        chat.activity.len().saturating_sub(available)
    } else {
        let half = available / 2;
        selected.saturating_sub(half)
    };
    let end = (start + available).min(chat.activity.len());

    let items = chat.activity
        .iter()
        .skip(start)
        .take(end - start)
        .map(|item| {
            let (tag, style) = match item {
                ActivityItem::CommandAccepted { .. } => ("cmd", Style::default().fg(theme::ACCENT)),
                ActivityItem::ToolCall { .. } => ("tool", Style::default().fg(theme::SECONDARY)),
                ActivityItem::ToolResult { ok, .. } => {
                    if *ok {
                        ("ok", Style::default().fg(theme::SUCCESS))
                    } else {
                        ("fail", Style::default().fg(theme::ERROR))
                    }
                }
                ActivityItem::OutputChunk { .. } => {
                    ("out", Style::default().add_modifier(Modifier::DIM))
                }
                ActivityItem::ReasoningChunk { .. } => (
                    "think",
                    Style::default().fg(theme::INFO).add_modifier(Modifier::DIM),
                ),
                ActivityItem::Warning { .. } => ("warn", Style::default().fg(theme::SECONDARY)),
                ActivityItem::Failure { .. } => ("error", Style::default().fg(theme::ERROR)),
                ActivityItem::RetryAttempt { .. } => {
                    ("retry", Style::default().fg(theme::SECONDARY))
                }
                ActivityItem::Completed => ("done", Style::default().fg(theme::SUCCESS)),
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
        .border_style(Style::default().fg(theme::SECONDARY));

    let mut lines = item.details_lines();
    lines.push(Line::raw(""));
    lines.push(Line::raw("Enter/Esc: close"));

    let modal = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}
