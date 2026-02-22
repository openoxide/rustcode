use std::time::{Duration, SystemTime};

use super::super::{Line, Modifier, Span, Style, ToastVariant};
use crate::interactive::theme;

const RUNNING_FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
const RUNNING_DOTS: &[&str] = &["   ", ".  ", ".. ", "..."];

/// Build the two footer lines (status + hints).
///
/// `available_width` controls progressive hint degradation on narrow terminals.
pub(super) fn build_footer_lines(
    is_running: bool,
    latest_toast: Option<(String, ToastVariant)>,
    composer_starts_with_query: bool,
    activity_hidden: bool,
    has_error: bool,
    available_width: u16,
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

    let w = available_width;

    let hints_line = if w < 30 {
        // Ultra-narrow: just the help shortcut.
        Line::from(vec![Span::styled(
            " ?:help",
            Style::default().fg(theme::MUTED),
        )])
    } else if composer_starts_with_query {
        build_query_hints(activity_hidden, has_error, w)
    } else {
        build_default_hints(activity_hidden, has_error, w)
    };

    (status_line, hints_line)
}

/// Hints shown when composer text starts with `?`.
fn build_query_hints(activity_hidden: bool, has_error: bool, w: u16) -> Line<'static> {
    let mut spans = vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            "Ctrl+P",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(":cmds  ", Style::default().fg(theme::MUTED)),
    ];
    if w >= 50 {
        spans.push(Span::styled(
            "Ctrl+N",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(":new  ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "Ctrl+Q",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            ":sessions  ",
            Style::default().fg(theme::MUTED),
        ));
        spans.push(Span::styled(
            "Ctrl+C",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(":cancel  ", Style::default().fg(theme::MUTED)));
    }
    if w >= 50 && !activity_hidden {
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
    if w >= 70 {
        spans.push(Span::styled(
            "  Ctrl+O:toggle tools",
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        ));
    }
    Line::from(spans)
}

/// Default hints shown when composer is empty or has normal text.
fn build_default_hints(activity_hidden: bool, has_error: bool, w: u16) -> Line<'static> {
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
    ];
    if w >= 50 {
        spans.push(Span::styled("   /", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(" cmd", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "   ? bindings",
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        ));
    }
    if w >= 70 {
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
    }
    spans.push(error_part);
    Line::from(spans)
}
