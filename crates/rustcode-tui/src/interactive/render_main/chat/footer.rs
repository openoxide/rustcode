use std::time::{Duration, SystemTime};

use super::super::{Line, Modifier, Span, Style, ToastVariant};
use crate::interactive::theme;

const RUNNING_FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
const RUNNING_DOTS: &[&str] = &["   ", ".  ", ".. ", "..."];

pub(super) fn build_footer_lines(
    is_running: bool,
    latest_toast: Option<(String, ToastVariant)>,
    composer_starts_with_query: bool,
    activity_hidden: bool,
    has_error: bool,
) -> (Line<'static>, Line<'static>) {
    let (status_text, status_style) = if let Some((message, variant)) = latest_toast {
        let style = match variant {
            ToastVariant::Info => Style::default().fg(theme::ACCENT),
            ToastVariant::Success => Style::default().fg(theme::SUCCESS),
            ToastVariant::Warning => Style::default().fg(theme::SECONDARY),
            ToastVariant::Error => Style::default().fg(theme::ERROR),
        };
        (message, style)
    } else {
        (String::new(), Style::default())
    };

    let error_hint = if has_error {
        Span::styled(
            "  Ctrl+E:details",
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };

    let mut status_spans = Vec::new();
    if is_running {
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();
        let frame_char = RUNNING_FRAMES[(ms / 120 % RUNNING_FRAMES.len() as u128) as usize];
        let dots = RUNNING_DOTS[(ms / 300 % RUNNING_DOTS.len() as u128) as usize];
        status_spans.push(Span::styled(
            format!(" {frame_char} running{dots}"),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    }
    status_spans.push(Span::styled(status_text, status_style));
    status_spans.push(error_hint);
    let status_line = Line::from(status_spans);

    let hints_line = if composer_starts_with_query {
        let mut spans = vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                "Ctrl+P",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cmds  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "Ctrl+N",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":new  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "Ctrl+Q",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":sessions  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "Ctrl+C",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cancel  ", Style::default().fg(theme::MUTED)),
        ];
        if !activity_hidden {
            spans.push(Span::styled(
                "Alt+Tab",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                ":switch focus  ",
                Style::default().fg(theme::MUTED),
            ));
            spans.push(Span::styled(
                "Ctrl+W",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                ":toggle activity  ",
                Style::default().fg(theme::MUTED),
            ));
        }
        spans.push(Span::styled(
            "/",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(if has_error {
            Span::styled(":cmd  Ctrl+E:error", Style::default().fg(theme::MUTED))
        } else {
            Span::styled(":cmd", Style::default().fg(theme::MUTED))
        });
        spans.push(Span::styled(
            "  Ctrl+O:toggle tools",
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        ));
        Line::from(spans)
    } else {
        let error_part = if has_error {
            Span::styled(
                "  Ctrl+E:error",
                Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let mut spans = vec![
            Span::styled(" Ctrl+P", Style::default().fg(theme::MUTED)),
            Span::styled(" cmds", Style::default().fg(theme::MUTED)),
            Span::styled("   /", Style::default().fg(theme::MUTED)),
            Span::styled(" cmd", Style::default().fg(theme::MUTED)),
            Span::styled(
                "   ? bindings",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            ),
        ];
        if activity_hidden {
            spans.push(Span::styled(
                "   Ctrl+W",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            ));
            spans.push(Span::styled(
                " open activity",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            ));
        } else {
            spans.push(Span::styled(
                "   Alt+Tab",
                Style::default().fg(theme::MUTED),
            ));
            spans.push(Span::styled(
                " switch focus",
                Style::default().fg(theme::MUTED),
            ));
            spans.push(Span::styled("   Ctrl+W", Style::default().fg(theme::MUTED)));
            spans.push(Span::styled(
                " toggle activity",
                Style::default().fg(theme::MUTED),
            ));
        }
        spans.push(error_part);
        Line::from(spans)
    };

    (status_line, hints_line)
}
