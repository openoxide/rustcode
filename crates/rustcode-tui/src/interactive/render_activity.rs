use super::{
    centered_rect, ActivityItem, Block, Borders, ChatFocus, ChatState, Clear, Color, Duration,
    Line, List, ListItem, Modifier, Paragraph, PendingApproval, Rect, Span, Style, SystemTime,
    Wrap,
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
        match (ms / 120) % 4 {
            0 => '|',
            1 => '/',
            2 => '-',
            _ => '\\',
        }
    } else {
        ' '
    };
    let title = if chat.running.is_some() {
        format!("Activity {spinner}")
    } else {
        "Activity".to_string()
    };

    if chat.activity.is_empty() {
        let empty = if chat.running.is_some() {
            "(running...)"
        } else {
            "(no activity yet)"
        };
        let list = List::new(vec![ListItem::new(empty)]).block(
            Block::default()
                .title(title)
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
                .title(title)
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
    lines.push(Line::from(vec![
        Span::raw("Enter/Esc: close"),
        Span::raw("  "),
        Span::raw("Alt+Tab/click: focus"),
    ]));

    let modal = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}

pub(super) fn render_approval_modal(frame: &mut ratatui::Frame<'_>, pending: &PendingApproval) {
    let area = centered_rect(80, 60, frame.area());
    frame.render_widget(Clear, area);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        "Tool approval required",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!("tool: {}", pending.request.tool)));
    lines.push(Line::raw(format!(
        "permission: {}",
        pending.request.permission
    )));
    lines.push(Line::raw(format!("target: {}", pending.request.pattern)));
    lines.push(Line::raw(""));
    lines.push(Line::raw(format!("reason: {}", pending.request.reason)));
    lines.push(Line::raw(""));

    let mut action_spans = vec![
        approval_button("[A] Approve once", Color::Green),
        Span::raw("  "),
    ];
    if is_edit_permission(&pending.request.permission) {
        action_spans.push(approval_button("[E] Approve all edits", Color::Cyan));
        action_spans.push(Span::raw("  "));
    }
    if is_command_permission(&pending.request.permission) {
        action_spans.push(approval_button("[C] Approve all commands", Color::Cyan));
        action_spans.push(Span::raw("  "));
    }
    action_spans.push(approval_button("[D] Deny", Color::Red));
    lines.push(Line::from(action_spans));
    lines.push(Line::from(vec![Span::raw("keys: 1=once  2=all  Esc=deny")]));
    lines.push(Line::raw(""));
    lines.push(Line::raw("arguments (preview):"));
    let args = serde_json::to_string_pretty(&pending.request.arguments)
        .unwrap_or_else(|_| "<unprintable>".to_string());
    let arg_lines = args.lines().collect::<Vec<_>>();
    for line in arg_lines.iter().take(6) {
        lines.push(Line::raw(truncate_modal_line(line, 100)));
    }
    if arg_lines.len() > 6 {
        lines.push(Line::raw("...[truncated]..."));
    }

    let block = Block::default()
        .title("Approval")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let modal = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(modal, area);
}

fn approval_button(label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(color)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED),
    )
}

fn truncate_modal_line(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = input
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}

fn is_edit_permission(permission: &str) -> bool {
    permission.eq_ignore_ascii_case("write") || permission.eq_ignore_ascii_case("edit")
}

fn is_command_permission(permission: &str) -> bool {
    permission.eq_ignore_ascii_case("exec")
}
