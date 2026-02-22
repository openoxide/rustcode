use crate::interactive::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::InteractiveSubmitMode;

use super::AppState;
use super::SLASH_COMMANDS;

/// Render the full keyboard-shortcut help overlay.
pub(crate) fn render_help_modal(frame: &mut ratatui::Frame<'_>, state: &AppState) {
    let area = centered_rect(80, 70, frame.area());
    frame.render_widget(Clear, area);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![Span::styled(
            "Help",
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
    ];

    lines.push(Line::from(vec![Span::styled(
        "Global",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw("  ?: toggle help"));
    lines.push(Line::raw("  Ctrl+P: command palette"));
    lines.push(Line::raw(
        "    - try: session <id>, rename <title>, search <text>",
    ));
    lines.push(Line::raw("  q: quit / back"));
    lines.push(Line::raw("  Esc: close modal / clear / back"));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Sessions",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw(
        "  Up/Down/PgUp/PgDn: select  Home/End: first/last",
    ));
    lines.push(Line::raw("  Enter: open session"));
    lines.push(Line::raw("  /: search/filter sessions"));
    lines.push(Line::raw("  Ctrl+N: new session"));
    lines.push(Line::raw("  Ctrl+E: rename session"));
    lines.push(Line::raw("  Ctrl+D: delete session"));
    lines.push(Line::raw("  Ctrl+R: refresh list"));
    lines.push(Line::raw("  Esc / Q: quit"));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![Span::styled(
        "Chat",
        Style::default().add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::raw(
        "  Alt+Tab: switch focus (Composer ↔ Activity, only when panel is visible)",
    ));
    lines.push(Line::raw(
        "  Esc: focus back to composer / clear composer / back to sessions",
    ));
    lines.push(Line::raw(
        "  Enter: submit prompt (composer) / open details (activity)",
    ));
    lines.push(Line::raw("  Shift+Enter: insert newline in composer"));
    lines.push(Line::raw("  Up/Down: browse prompt history (composer)"));
    lines.push(Line::raw("  Alt+Up/Down: move cursor line in composer"));
    lines.push(Line::raw(
        "  Ctrl+K: kill to end of line  Ctrl+U: kill to start of line",
    ));
    lines.push(Line::raw("  Ctrl+Left/Right: jump word in composer"));
    lines.push(Line::raw("  Up/Down: select item (activity)"));
    lines.push(Line::raw("  PgUp/PgDn: scroll transcript"));
    lines.push(Line::raw(
        "  / (any focus): move to composer and insert / for slash commands",
    ));
    lines.push(Line::raw("  Ctrl+C: cancel running agent"));
    lines.push(Line::raw("  Ctrl+N: new session"));
    lines.push(Line::raw("  Ctrl+F: fork current session"));
    lines.push(Line::raw("  Ctrl+R: refresh transcript"));
    lines.push(Line::raw(
        "  Ctrl+T: file search (inserts @path into composer)",
    ));
    lines.push(Line::raw("  Ctrl+Q: go to sessions list"));
    lines.push(Line::raw("  Ctrl+P: open command palette"));
    lines.push(Line::raw("  Ctrl+M: switch model (pick a different LLM)"));
    lines.push(Line::raw("  Ctrl+A: manage providers (connect/disconnect)"));
    lines.push(Line::raw("  Ctrl+S: skill toggle overlay"));
    lines.push(Line::raw("  Ctrl+B: give feedback (thumbs up/down)"));
    lines.push(Line::raw("  Ctrl+W: toggle activity panel (show/hide)"));
    lines.push(Line::raw("  Ctrl+O: expand/collapse tool details"));
    lines.push(Line::raw(""));
    lines.push(Line::raw(
        "  Slash commands (type in composer, press Enter):",
    ));
    lines.push(Line::raw(
        "  /help  /sessions  /new  /fork  /reload  /find  /model  /providers  /clear  /skill  /memory",
    ));

    if state.submit_mode == InteractiveSubmitMode::Run {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled(
            "Attach mode",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::raw("  This TUI is attached to a remote server."));
        lines.push(Line::raw("  Tool approvals may not be supported."));
    }

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::ACCENT)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(dialog, area);
}

/// Render the slash-command autocomplete popup.
pub(super) fn render_slash_help(frame: &mut ratatui::Frame<'_>, query: &str, selected: usize) {
    let filtered: Vec<(&str, &str)> = if query.is_empty() {
        SLASH_COMMANDS.to_vec()
    } else {
        let needle = query.to_ascii_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| {
                let stem = cmd
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                stem.to_ascii_lowercase().contains(&needle)
            })
            .copied()
            .collect()
    };

    let area = slash_popup_rect(frame.area(), filtered.len());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("/ commands  (Tab: complete  Esc: close)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if filtered.is_empty() {
        let msg =
            Paragraph::new("(no matches)").style(Style::default().add_modifier(Modifier::DIM));
        frame.render_widget(msg, inner);
        return;
    }

    let clamped = selected.min(filtered.len().saturating_sub(1));
    let items: Vec<ListItem<'_>> = filtered
        .iter()
        .enumerate()
        .map(|(i, (cmd, desc))| {
            let style = if i == clamped {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(*cmd, style.fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled("  ", style),
                Span::styled(*desc, style.fg(theme::MUTED)),
            ]))
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(clamped));
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, inner, &mut list_state);
}

/// Compute the popup rect for the slash-help popup.
///
/// Anchored to the lower-left of the terminal, just above the composer+footer
/// (~8 rows from the bottom).  Width is half the terminal; height grows with
/// the number of matches up to 10 rows.
fn slash_popup_rect(r: Rect, item_count: usize) -> Rect {
    let content_h = (item_count as u16).clamp(1, 10);
    // +2 for borders, clamp to available terminal height
    let h = (content_h + 2).min(r.height.saturating_sub(2));
    let w = (r.width / 2).max(20.min(r.width));
    // Position above the composer+footer (~8 rows from bottom), clamped to r.y
    let y = r.y + r.height.saturating_sub(h + 8);
    let y = y.max(r.y);
    Rect {
        x: r.x + 1,
        y,
        width: w,
        height: h,
    }
}

/// Compute a centered rectangle within `r` using percentage dimensions.
///
/// Clamps the result so it never produces a zero-width or zero-height rect
/// and always fits within the container, even on very small terminals.
pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let w = ((r.width as u32 * percent_x as u32) / 100) as u16;
    let h = ((r.height as u32 * percent_y as u32) / 100) as u16;
    // Ensure minimum usable size, clamped to the container.
    let w = w.max(20.min(r.width)).min(r.width);
    let h = h.max(5.min(r.height)).min(r.height);
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    let y = r.y + (r.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
