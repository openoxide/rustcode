use super::super::{
    Block, Borders, Clear, Constraint, Direction, Layout, Line, Modal, Modifier, Paragraph, Span,
    Style, Wrap,
};
use super::centered_rect;
use crate::interactive::markdown::render_markdown;
use crate::interactive::theme;

/// Render the memory viewer and memory clear confirmation modals.
pub(in crate::interactive) fn render_memory_modal(frame: &mut ratatui::Frame<'_>, modal: &Modal) {
    match modal {
        Modal::MemoryViewer {
            content,
            raw_count,
            updated_at,
            enabled,
            scroll,
            total_lines,
        } => {
            let area = centered_rect(85, 80, frame.area());
            frame.render_widget(Clear, area);

            let status_label = if *enabled { "enabled" } else { "disabled" };
            let block = Block::default()
                .title(format!("Memory  [{status_label}]"))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::SECONDARY));
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

            // Header: status info
            let updated_str = if *updated_at == 0 {
                "never".to_string()
            } else {
                let secs_ago = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs().saturating_sub(*updated_at))
                    .unwrap_or(0);
                if secs_ago < 60 {
                    format!("{secs_ago}s ago")
                } else if secs_ago < 3600 {
                    format!("{}m ago", secs_ago / 60)
                } else {
                    format!("{}h ago", secs_ago / 3600)
                }
            };
            let header = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        status_label,
                        Style::default().fg(if *enabled {
                            theme::SUCCESS
                        } else {
                            theme::WARNING
                        }),
                    ),
                    Span::raw("    "),
                    Span::styled(
                        "Raw memories: ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(raw_count.to_string()),
                    Span::raw("    "),
                    Span::styled(
                        "Last updated: ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(updated_str),
                ]),
                Line::raw(""),
                Line::styled(
                    "Memories are facts extracted from past sessions and injected into the system prompt.",
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]);
            frame.render_widget(header, rows[0]);

            // Body: render content as styled markdown, then display with scroll
            let md_lines = render_markdown(content);
            let scroll_info = if *total_lines > 0 {
                format!(" ({}/{})", scroll + 1, total_lines)
            } else {
                String::new()
            };
            let body = Paragraph::new(md_lines)
                .block(
                    Block::default()
                        .title(format!("Summary{scroll_info}"))
                        .borders(Borders::ALL),
                )
                .scroll((*scroll as u16, 0))
                .wrap(Wrap { trim: false });
            frame.render_widget(body, rows[1]);

            // Footer: hints
            let hint = Paragraph::new(Line::from(vec![
                Span::raw("Up/Down/j/k: scroll  "),
                Span::raw("t: toggle on/off  "),
                Span::raw("Esc: close"),
            ]))
            .block(Block::default().borders(Borders::TOP));
            frame.render_widget(hint, rows[2]);
        }
        Modal::MemoryClearConfirm => {
            let area = centered_rect(60, 25, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title("Clear memories")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::WARNING));
            let lines = vec![
                Line::raw("This will delete all raw memories AND the consolidated summary."),
                Line::raw(""),
                Line::raw("This action cannot be undone."),
                Line::raw(""),
                Line::raw("y: clear all    n/Esc: cancel"),
            ];
            frame.render_widget(Paragraph::new(lines).block(block), area);
        }
        _ => {}
    }
}
