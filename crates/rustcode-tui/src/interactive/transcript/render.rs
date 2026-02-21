//! Tool output rendering and live-activity feed for the transcript pane.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::syntax_highlight::{highlight_code_line, HighlightState};
use super::super::ActivityItem;
use super::helpers::{
    extract_filename, extract_key_arg, extract_tool_output_text, is_stat_line, parse_exit_code,
    parse_hunk_header, sanitize_output_line, tilde_path_in_line,
};
use crate::interactive::theme;

const LIVE_ACTIVITY_TRI: &[char] = &['▶', '▸', '▹', '▻'];

/// Summarise a tool result into a single compact line of text.
pub(super) fn summarize_tool_result(tool_name: &str, output: &str, ok: bool) -> String {
    let tool = tool_name.to_ascii_lowercase();
    let nonempty_lines = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let clean_first_line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| sanitize_output_line(&tilde_path_in_line(line)))
        .unwrap_or_default();

    match tool.as_str() {
        "read" => format!(
            "Read {nonempty_lines} {}",
            if nonempty_lines == 1 { "line" } else { "lines" }
        ),
        "list" | "glob" => format!(
            "Found {nonempty_lines} {}",
            if nonempty_lines == 1 { "file" } else { "files" }
        ),
        "grep" | "codesearch" => format!(
            "Found {nonempty_lines} {}",
            if nonempty_lines == 1 {
                "match"
            } else {
                "matches"
            }
        ),
        "exec" | "bash" | "pty_exec" => {
            if let Some(code) = parse_exit_code(output) {
                format!("Command exited {code}")
            } else if ok {
                "Command completed".to_string()
            } else {
                "Command failed".to_string()
            }
        }
        "write" | "edit" | "multiedit" | "apply_patch" => {
            if let Some(summary) = output.lines().find(|line| !line.trim().is_empty()) {
                sanitize_output_line(&tilde_path_in_line(summary))
            } else if ok {
                "Update applied".to_string()
            } else {
                "Update failed".to_string()
            }
        }
        _ => {
            if clean_first_line.is_empty() {
                if ok {
                    "Done".to_string()
                } else {
                    "Failed".to_string()
                }
            } else {
                clean_first_line.chars().take(96).collect()
            }
        }
    }
}

/// Render tool output with GitHub-patch-style diff rendering.
///
/// Diff output gets a file header, coloured stat, and left-border pipe (`│`)
/// on every diff line for a clean patch-like appearance.
///
/// Plain output: collapsed shows first non-empty line; expanded shows up to 20 lines.
pub(super) fn render_tool_output(
    lines: &mut Vec<Line<'static>>,
    tool_name: &str,
    output: &str,
    expanded: bool,
    ok: bool,
) {
    const DIFF_MARKER: &str = "\n@@diff\n";
    if let Some(diff_pos) = output.find(DIFF_MARKER) {
        let summary = output[..diff_pos].trim();
        let diff_body = &output[diff_pos + DIFF_MARKER.len()..];

        let mut desc_line = String::new();
        let mut stat_line = String::new();
        for raw_line in summary.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if is_stat_line(trimmed) {
                stat_line = trimmed.to_string();
            } else {
                desc_line = trimmed.to_string();
            }
        }

        let filename = extract_filename(&desc_line);

        let mut header_spans: Vec<Span<'static>> =
            vec![Span::styled(" ┌─ ", Style::default().fg(theme::MUTED))];
        header_spans.push(Span::styled(
            filename,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ));
        if !stat_line.is_empty() {
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled("(", Style::default().fg(theme::MUTED)));
            for (i, word) in stat_line.split_whitespace().enumerate() {
                if i > 0 {
                    header_spans.push(Span::raw(" "));
                }
                let style = if word.starts_with('+') {
                    Style::default().fg(theme::SUCCESS)
                } else if word.starts_with('-') {
                    Style::default().fg(theme::ERROR)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                header_spans.push(Span::styled(word.to_string(), style));
            }
            header_spans.push(Span::styled(")", Style::default().fg(theme::MUTED)));
        }
        lines.push(Line::from(header_spans));

        if expanded {
            let ext = desc_line
                .split_whitespace()
                .rev()
                .find(|t| t.contains('/'))
                .and_then(|p| p.rsplit('.').next())
                .unwrap_or("")
                .to_string();
            let mut hl = HighlightState::new(&ext);

            let mut add_no = 0usize;
            let mut del_no = 0usize;
            for raw in diff_body.lines() {
                if raw.starts_with("@@") {
                    if let Some(range) = parse_hunk_header(raw) {
                        add_no = range.0;
                        del_no = range.1;
                    }
                    lines.push(Line::from(Span::styled(
                        " │ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄".to_string(),
                        Style::default().fg(theme::SURFACE),
                    )));
                    continue;
                }
                if raw.starts_with("+++") || raw.starts_with("---") {
                    continue;
                }

                let is_add = raw.starts_with('+');
                let is_del = raw.starts_with('-');
                let is_diff_line = is_add || is_del;

                let (gutter, content_style, pipe_color) = if is_add {
                    add_no += 1;
                    (
                        format!("{add_no:>2} "),
                        Style::default()
                            .fg(theme::DIFF_ADD_FG)
                            .bg(theme::DIFF_ADD_BG),
                        theme::DIFF_ADD_FG,
                    )
                } else if is_del {
                    del_no += 1;
                    (
                        format!("{del_no:>2} "),
                        Style::default()
                            .fg(theme::DIFF_DEL_FG)
                            .bg(theme::DIFF_DEL_BG),
                        theme::DIFF_DEL_FG,
                    )
                } else {
                    add_no += 1;
                    del_no += 1;
                    (
                        format!("{add_no:>2} "),
                        Style::default().fg(theme::TEXT_DIM),
                        theme::MUTED,
                    )
                };

                let code_text = raw.get(1..).unwrap_or(raw);
                let sign = if is_add {
                    "+"
                } else if is_del {
                    "-"
                } else {
                    ""
                };

                if is_diff_line {
                    let mut line_spans = vec![
                        Span::styled(" │ ", Style::default().fg(pipe_color)),
                        Span::styled(gutter, Style::default().fg(pipe_color)),
                        Span::styled(sign.to_string(), Style::default().fg(pipe_color)),
                    ];
                    if let Some(ref mut state) = hl {
                        let hl_spans = highlight_code_line(code_text, state);
                        if hl_spans.is_empty() {
                            line_spans.push(Span::styled(code_text.to_string(), content_style));
                        } else {
                            line_spans.extend(hl_spans);
                        }
                    } else {
                        line_spans.push(Span::styled(code_text.to_string(), content_style));
                    }
                    let bg = if is_add {
                        Style::default().bg(theme::DIFF_ADD_BG)
                    } else {
                        Style::default().bg(theme::DIFF_DEL_BG)
                    };
                    lines.push(Line::from(line_spans).style(bg));
                } else {
                    let mut line_spans = vec![
                        Span::styled(" │ ", Style::default().fg(pipe_color)),
                        Span::styled(gutter, Style::default().fg(theme::MUTED)),
                    ];
                    if let Some(rest) = raw.strip_prefix(' ') {
                        line_spans.push(Span::styled(" ", Style::default()));
                        if let Some(ref mut state) = hl {
                            let hl_spans = highlight_code_line(rest, state);
                            if hl_spans.is_empty() {
                                line_spans.push(Span::styled(rest.to_string(), content_style));
                            } else {
                                line_spans.extend(hl_spans);
                            }
                        } else {
                            line_spans.push(Span::styled(rest.to_string(), content_style));
                        }
                    } else {
                        line_spans.push(Span::styled(raw.to_string(), content_style));
                    }
                    lines.push(Line::from(line_spans));
                }
            }
        }

        lines.push(Line::from(Span::styled(
            " └─",
            Style::default().fg(theme::MUTED),
        )));
    } else {
        let summary = summarize_tool_result(tool_name, output, ok);
        if !summary.is_empty() {
            let style = if ok {
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM)
            } else {
                Style::default().fg(theme::DIFF_DEL_FG)
            };
            lines.push(Line::from(Span::styled(format!("  └  {summary}"), style)));
        }
        if expanded {
            lines.push(Line::from(Span::styled(
                "  └─",
                Style::default().fg(theme::MUTED),
            )));
        }
    }
}

/// Render a compact summary line of live tool calls (collapsed mode).
pub(super) fn render_live_activity_summary(
    lines: &mut Vec<Line<'static>>,
    activity: &[ActivityItem],
    ms: u128,
) {
    let tool_calls: Vec<(&str, &str)> = activity
        .iter()
        .filter_map(|item| match item {
            ActivityItem::ToolCall {
                name, arguments, ..
            } => Some((name.as_str(), arguments.as_str())),
            _ => None,
        })
        .collect();
    if tool_calls.is_empty() {
        return;
    }

    let (last_name, last_args) = tool_calls[tool_calls.len() - 1];
    let key_arg = extract_key_arg(last_name, last_args);
    let latest = if key_arg.is_empty() {
        last_name.to_string()
    } else {
        format!("{last_name}({key_arg})")
    };
    let more = tool_calls.len().saturating_sub(1);
    let more_part = if more > 0 {
        format!(
            "  +{more} more tool use{}",
            if more == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };

    let tri = LIVE_ACTIVITY_TRI[(ms / 120 % LIVE_ACTIVITY_TRI.len() as u128) as usize];
    lines.push(Line::from(Span::styled(
        format!("  {tri} {latest}{more_part}  (ctrl+o to expand)"),
        Style::default().fg(theme::ACTIVITY_DONE),
    )));
}

/// Render a detailed live feed of tool calls while a run is in progress.
pub(super) fn render_live_activity_lines(
    lines: &mut Vec<Line<'static>>,
    activity: &[ActivityItem],
    ms: u128,
) {
    const SPIN: &[char] = &['⠋', '⠙', '⠸', '⠴', '⠦', '⠇'];
    let spin_frame = SPIN[(ms / 100 % 6) as usize];

    let mut results: HashMap<&str, (bool, &str)> = HashMap::new();
    for item in activity {
        if let ActivityItem::ToolResult { id, ok, output, .. } = item {
            results.insert(id.as_str(), (*ok, output.as_str()));
        }
    }

    let displayable: Vec<&ActivityItem> = activity
        .iter()
        .filter(|item| {
            matches!(
                item,
                ActivityItem::ToolCall { .. }
                    | ActivityItem::Warning { .. }
                    | ActivityItem::Failure { .. }
            )
        })
        .collect();
    let show_from = displayable.len().saturating_sub(8);
    let show = &displayable[show_from..];

    let out_style = Style::default()
        .fg(theme::MUTED)
        .add_modifier(Modifier::DIM);

    for item in show {
        match item {
            ActivityItem::ToolCall {
                id,
                name,
                arguments,
            } => {
                let result = results.get(id.as_str());
                let is_pending = result.is_none();
                let is_ok = result.is_some_and(|(ok, _)| *ok);

                let name_cap: String = {
                    let mut c = name.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    }
                };

                let key_arg = extract_key_arg(name, arguments);
                let header_text = if key_arg.is_empty() {
                    name_cap
                } else {
                    format!("{name_cap}({key_arg})")
                };

                let (bullet, bullet_style) = if is_pending {
                    (format!("{spin_frame} "), Style::default().fg(theme::MUTED))
                } else if is_ok {
                    ("● ".to_string(), Style::default().fg(theme::SUCCESS))
                } else {
                    ("● ".to_string(), Style::default().fg(theme::ERROR))
                };
                let header_style = if is_pending {
                    Style::default().fg(theme::ACTIVITY_PENDING)
                } else {
                    Style::default()
                        .fg(theme::ACTIVITY_DONE)
                        .add_modifier(Modifier::DIM)
                };
                lines.push(Line::from(vec![
                    Span::styled(bullet, bullet_style),
                    Span::styled(header_text, header_style),
                ]));

                if let Some((_, output)) = result {
                    let decoded = extract_tool_output_text(output);
                    let summary = summarize_tool_result(name, &decoded, is_ok);
                    if !summary.is_empty() {
                        lines.push(Line::from(Span::styled(format!("└  {summary}"), out_style)));
                    }
                }
            }
            ActivityItem::Warning { message } => {
                let capped: String = message.chars().take(70).collect();
                lines.push(Line::from(Span::styled(
                    format!("⚠ {capped}"),
                    Style::default()
                        .fg(theme::ACTIVITY_WARN)
                        .add_modifier(Modifier::DIM),
                )));
            }
            ActivityItem::Failure { message } => {
                let capped: String = message.chars().take(70).collect();
                lines.push(Line::from(Span::styled(
                    format!("✗ {capped}"),
                    Style::default()
                        .fg(theme::ACTIVITY_FAIL)
                        .add_modifier(Modifier::DIM),
                )));
            }
            _ => {}
        }
    }
}
