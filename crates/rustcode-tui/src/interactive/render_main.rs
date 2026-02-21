use super::{
    format_age, render_help_modal, render_modal, AppState, Block, Borders, Clear, Constraint,
    Direction, Layout, Line, List, ListItem, Modifier, Paragraph, Screen, Span, Style,
    ToastVariant,
};
use crate::interactive::theme;

mod chat;
use self::chat::render_chat;

pub(super) fn render(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    frame.render_widget(Clear, frame.area());

    match &state.screen {
        Screen::Sessions => render_sessions(frame, state),
        Screen::Chat(chat) => render_chat(frame, state, chat),
    }

    if let Some(modal) = &state.modal {
        render_modal(frame, modal);
    }
    if state.help_open {
        render_help_modal(frame, state);
    }
}

pub(super) fn render_sessions(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(frame.area());

    let items = if state.sessions_view.is_empty() {
        if state.sessions_filter.trim().is_empty() {
            vec![ListItem::new("(no sessions yet - press n to create one)")]
        } else {
            vec![ListItem::new("(no matches)")]
        }
    } else {
        state
            .sessions_view
            .iter()
            .filter_map(|idx| state.sessions.get(*idx))
            .map(|session| {
                let age = format_age(session.updated_at_unix_ms);
                let fork_marker = if session.parent_id.is_some() {
                    "⑂ "
                } else {
                    "  "
                };
                let id_short = &session.id;
                let title = session.title.as_deref().unwrap_or("").trim();
                let mut spans = vec![Span::styled(
                    fork_marker,
                    Style::default().fg(theme::ACCENT),
                )];
                if !title.is_empty() {
                    spans.push(Span::styled(
                        title.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::raw("  "));
                }
                spans.extend([
                    Span::styled(
                        id_short.clone(),
                        Style::default()
                            .fg(theme::MUTED)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::raw("  "),
                    Span::styled(age, Style::default().add_modifier(Modifier::DIM)),
                ]);
                if !session.branch.trim().is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        format!("on {}", session.branch.trim()),
                        Style::default()
                            .fg(theme::MUTED)
                            .add_modifier(Modifier::DIM),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(format!("Sessions ({})", state.sessions_view.len()))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ratatui::widgets::ListState::default();
    if !state.sessions_view.is_empty() {
        let idx = state
            .selected
            .min(state.sessions_view.len().saturating_sub(1));
        list_state.select(Some(idx));
    }
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    // ─── Footer: 2-line layout matching chat screen ───────────────────────────
    let model_label = state.defaults.model.as_str();

    // Line 1: contextual status — filter mode, error/toast, or just model label
    let (main_text, main_style) = if state.sessions_filter_active {
        let text = if state.sessions_filter.is_empty() {
            " filter: ".to_string()
        } else {
            format!(" filter: {}", state.sessions_filter)
        };
        (
            text,
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if !state.sessions_filter.is_empty() {
        (
            format!(" filter:{}", state.sessions_filter),
            Style::default().fg(theme::ACCENT),
        )
    } else if let Some(toast) = state.toasts.last() {
        let style = match toast.variant {
            ToastVariant::Info => Style::default().fg(theme::ACCENT),
            ToastVariant::Success => Style::default().fg(theme::SUCCESS),
            ToastVariant::Warning => Style::default().fg(theme::SECONDARY),
            ToastVariant::Error => Style::default().fg(theme::ERROR),
        };
        (format!(" {}", toast.message), style)
    } else {
        (String::new(), Style::default())
    };
    let status_line = Line::from(vec![
        Span::styled(main_text, main_style),
        Span::styled(
            format!("  [{model_label}]"),
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        ),
    ]);

    // Line 2: key hints — filter-mode aware, styled like chat screen compact hints
    let hints_line = if state.sessions_filter_active {
        Line::from(vec![
            Span::styled(
                " Enter",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":open  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":clear  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "↑↓",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":select", Style::default().fg(theme::MUTED)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Ctrl+P", Style::default().fg(theme::MUTED)),
            Span::styled(" cmds", Style::default().fg(theme::MUTED)),
            Span::styled("   /", Style::default().fg(theme::MUTED)),
            Span::styled(" filter", Style::default().fg(theme::MUTED)),
            Span::styled("   N", Style::default().fg(theme::MUTED)),
            Span::styled(" new", Style::default().fg(theme::MUTED)),
            Span::styled("   E", Style::default().fg(theme::MUTED)),
            Span::styled(" rename", Style::default().fg(theme::MUTED)),
            Span::styled("   D", Style::default().fg(theme::MUTED)),
            Span::styled(" delete", Style::default().fg(theme::MUTED)),
            Span::styled(
                "   ? help",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            ),
        ])
    };

    let footer = Paragraph::new(vec![status_line, hints_line]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::BORDER)),
    );
    frame.render_widget(footer, chunks[1]);
}

/// Format a number with comma separators (e.g., 19674 → "19,674").
fn format_tokens(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }

    let keep = max_chars - 1;
    let head = keep / 2;
    let tail = keep - head;
    let prefix: String = text.chars().take(head).collect();
    let suffix: String = text
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}
