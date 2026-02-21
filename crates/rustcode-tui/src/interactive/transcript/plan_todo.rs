//! Plan and todo live widget rendering for the transcript.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::ChatState;

/// Append live plan widget lines to the transcript.
///
/// Renders the current plan as a titled checklist with coloured status icons.
pub(super) fn render_plan_lines(lines: &mut Vec<Line<'static>>, chat: &ChatState) {
    let Some(title) = &chat.plan_title else {
        return;
    };
    if chat.plan_steps.is_empty() {
        return;
    }

    lines.push(Line::from(vec![
        Span::styled("📋 ", Style::default()),
        Span::styled(
            title.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    for (idx, (desc, status)) in chat.plan_steps.iter().enumerate() {
        let (icon, icon_color) = match status.as_str() {
            "completed" => ("✅", Color::Green),
            "in_progress" => ("🔄", Color::Yellow),
            "blocked" => ("🚫", Color::Red),
            _ => ("⬜", Color::Rgb(100, 105, 120)),
        };
        let desc_style = if status == "completed" {
            Style::default()
                .fg(Color::Rgb(100, 110, 130))
                .add_modifier(Modifier::DIM)
        } else if status == "in_progress" {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::Rgb(180, 185, 200))
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {icon} "), Style::default().fg(icon_color)),
            Span::styled(format!("{}. {desc}", idx + 1), desc_style),
        ]));
    }
    lines.push(Line::raw(""));
}

/// Append live todo widget lines to the transcript.
///
/// Renders the current todos as a prioritized list with status indicators.
pub(super) fn render_todo_lines(lines: &mut Vec<Line<'static>>, chat: &ChatState) {
    if chat.todos.is_empty() {
        return;
    }

    lines.push(Line::from(vec![
        Span::styled("📝 ", Style::default()),
        Span::styled(
            "Todo List",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    for (content, status, priority) in &chat.todos {
        let (icon, icon_color) = match status.as_str() {
            "completed" => ("✓", Color::Green),
            "in_progress" => ("◐", Color::Yellow),
            "cancelled" => ("✗", Color::Red),
            _ => ("○", Color::Rgb(100, 105, 120)),
        };
        let content_style = if status == "completed" {
            Style::default()
                .fg(Color::Rgb(100, 110, 130))
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(Color::Rgb(180, 185, 200))
        };
        let priority_label = match priority.as_str() {
            "high" => Span::styled(
                " [HIGH]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            "medium" => Span::styled(" [MED]", Style::default().fg(Color::Yellow)),
            _ => Span::raw(""),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {icon} "), Style::default().fg(icon_color)),
            Span::styled(content.clone(), content_style),
            priority_label,
        ]));
    }
    lines.push(Line::raw(""));
}
