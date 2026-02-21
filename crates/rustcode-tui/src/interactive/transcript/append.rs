//! Message and tool-output appending for the transcript render pipeline.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use rustcode_core::{MessageRole, StoredMessage};

use super::super::markdown::render_markdown;
use super::super::syntax_highlight::{highlight_code_line, HighlightState};
use super::helpers::{extract_key_arg, is_editing_tool, parse_tool_payload, tilde_path_in_line};
use super::render::render_tool_output;

pub(super) fn append_value_lines(
    lines: &mut Vec<Line<'static>>,
    value: &serde_json::Value,
    prefix: &str,
    max_lines: usize,
) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::String(text) => {
            append_text_lines(lines, text.as_str(), prefix, max_lines);
        }
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string());
            append_text_lines(lines, pretty.as_str(), prefix, max_lines);
        }
    }
}

pub(super) fn append_text_lines(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    prefix: &str,
    max_lines: usize,
) {
    if text.is_empty() {
        return;
    }
    let mut in_code = false;
    for (idx, line) in text.lines().enumerate() {
        if idx >= max_lines {
            lines.push(Line::raw(format!("{prefix}...[truncated]...")));
            break;
        }

        let rendered = if prefix.is_empty() {
            line.to_string()
        } else {
            format!("{prefix}{line}")
        };

        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default().add_modifier(Modifier::DIM),
            )));
            continue;
        }

        if in_code {
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default().add_modifier(Modifier::DIM),
            )));
            continue;
        }

        if trimmed.starts_with('#') {
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default().add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        lines.push(Line::raw(rendered));
    }
}

/// Append contextual spans to the tool result header based on tool type and args.
///
/// For example: `✓ bash  $ cargo build`, `✓ glob  *.py`, `✓ websearch  "rust TUI"`.
pub(super) fn append_tool_context(
    spans: &mut Vec<Span<'static>>,
    tool: &str,
    args: Option<&serde_json::Value>,
) {
    let get = |keys: &[&str]| -> Option<String> {
        let a = args?;
        for k in keys {
            if let Some(v) = a.get(*k).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    };

    let short = |s: &str, max: usize| -> (String, bool) {
        let t: String = s.chars().take(max).collect();
        let truncated = t.len() < s.chars().count();
        (t, truncated)
    };

    match tool {
        "bash" | "exec" | "pty_exec" => {
            if let Some(cmd) = get(&["command", "cmd"]) {
                let (s, trunc) = short(&cmd, 60);
                spans.push(Span::styled("  $ ", Style::default().fg(Color::DarkGray)));
                let mut hl_bash = HighlightState::new("sh");
                if let Some(ref mut state) = hl_bash {
                    let hl_spans = highlight_code_line(&s, state);
                    if hl_spans.is_empty() {
                        spans.push(Span::styled(s, Style::default().fg(Color::DarkGray)));
                    } else {
                        spans.extend(hl_spans);
                    }
                } else {
                    spans.push(Span::styled(s, Style::default().fg(Color::DarkGray)));
                }
                if trunc {
                    spans.push(Span::styled("…", Style::default().fg(Color::DarkGray)));
                }
            }
        }
        "glob" => {
            if let Some(pat) = get(&["pattern"]) {
                let (s, _) = short(&pat, 40);
                spans.push(Span::styled(
                    format!("  {s}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "grep" => {
            if let Some(pat) = get(&["pattern"]) {
                let (s, _) = short(&pat, 40);
                spans.push(Span::styled(
                    format!("  /{s}/"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "codesearch" => {
            if let Some(q) = get(&["query"]) {
                let (s, _) = short(&q, 40);
                spans.push(Span::styled(
                    format!("  \"{s}\""),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "read" | "list" => {
            if let Some(p) = get(&["path"]) {
                let display = tilde_path_in_line(&p);
                let (s, trunc) = short(&display, 50);
                spans.push(Span::styled(
                    format!("  {s}{}", if trunc { "…" } else { "" }),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "websearch" => {
            if let Some(q) = get(&["query"]) {
                let (s, _) = short(&q, 40);
                spans.push(Span::styled(
                    format!("  \"{s}\""),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "webfetch" => {
            if let Some(url) = get(&["url"]) {
                let (s, trunc) = short(&url, 50);
                spans.push(Span::styled(
                    format!("  {s}{}", if trunc { "…" } else { "" }),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "plan" => {
            if let Some(t) = get(&["title"]) {
                let (s, _) = short(&t, 40);
                spans.push(Span::styled(
                    format!("  {s}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "task" => {
            if let Some(p) = get(&["prompt"]) {
                let (s, _) = short(&p, 40);
                spans.push(Span::styled(
                    format!("  {s}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "skill" => {
            if let Some(n) = get(&["name"]) {
                spans.push(Span::styled(
                    format!("  {n}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        "lsp" => {
            if let Some(op) = get(&["operation"]) {
                spans.push(Span::styled(
                    format!("  {op}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        _ => {}
    }
}

pub(super) fn append_tool_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &StoredMessage,
    output_details: bool,
    tool_args_map: &HashMap<String, String>,
) {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    let content_str = msg.content.as_str().unwrap_or("");

    if let Some((ok, _truncated, output)) = parse_tool_payload(content_str) {
        let (indicator, indicator_style) = if ok {
            (
                "✓",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "✗",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        };

        let mut header_spans = vec![Span::styled(
            format!("  {indicator} {name}"),
            indicator_style,
        )];

        let args_json = msg
            .tool_call_id
            .as_ref()
            .and_then(|id| tool_args_map.get(id))
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

        append_tool_context(&mut header_spans, name, args_json.as_ref());

        lines.push(Line::from(header_spans));

        let always_show_edit_diff = is_editing_tool(name) && output.contains("\n@@diff\n") && ok;
        render_tool_output(
            lines,
            name,
            &output,
            output_details || always_show_edit_diff,
            ok,
        );
        return;
    }

    lines.push(Line::from(vec![Span::styled(
        format!("  ▶ {name}"),
        Style::default().fg(Color::Cyan),
    )]));
    if !content_str.is_empty() {
        lines.push(Line::from(Span::styled(
            "  └  Result available",
            Style::default()
                .fg(Color::Rgb(120, 125, 140))
                .add_modifier(Modifier::DIM),
        )));
    }
}

pub(super) fn append_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &StoredMessage,
    tool_details: bool,
    output_details: bool,
    tool_args_map: &HashMap<String, String>,
    show_reasoning: bool,
) {
    if msg.role == MessageRole::Tool {
        append_tool_message_lines(lines, msg, output_details, tool_args_map);
        return;
    }

    match msg.role {
        MessageRole::User => {
            let bg = Style::default().bg(Color::Rgb(25, 45, 80));
            match msg.content.as_str() {
                Some(text) if !text.is_empty() => {
                    let rendered = render_markdown(text);
                    let mut truncated = if rendered.len() > 400 {
                        let mut t = rendered[..400].to_vec();
                        t.push(Line::raw("...[truncated]..."));
                        t
                    } else {
                        rendered
                    };
                    if !truncated.is_empty() {
                        let idx = truncated.iter().position(|l| l.width() > 0).unwrap_or(0);
                        if idx < truncated.len() {
                            let mut spans = vec![Span::styled(
                                "</> ",
                                Style::default().fg(Color::Rgb(120, 200, 200)),
                            )];
                            spans.extend(truncated[idx].spans.iter().cloned());
                            truncated[idx] = Line::from(spans);
                        }
                    }
                    lines.extend(truncated.into_iter().map(|l| l.style(bg)));
                }
                _ => {
                    append_value_lines(lines, &msg.content, "", 200);
                }
            }
        }
        MessageRole::Assistant => {
            if let Some(reasoning) = msg
                .reasoning
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                if show_reasoning {
                    let rendered = render_markdown(reasoning);
                    let mut truncated = if rendered.len() > 300 {
                        let mut t = rendered[..300].to_vec();
                        t.push(Line::raw("...[thinking truncated]..."));
                        t
                    } else {
                        rendered
                    };
                    if !truncated.is_empty() {
                        let idx = truncated.iter().position(|l| l.width() > 0).unwrap_or(0);
                        if idx < truncated.len() {
                            let mut spans = vec![Span::styled(
                                "◦  ",
                                Style::default().fg(Color::Blue).add_modifier(Modifier::DIM),
                            )];
                            spans.extend(truncated[idx].spans.iter().cloned());
                            truncated[idx] = Line::from(spans);
                        }
                    }
                    lines.extend(truncated.into_iter().map(|line| {
                        line.style(
                            Style::default()
                                .fg(Color::Rgb(130, 140, 170))
                                .add_modifier(Modifier::DIM),
                        )
                    }));
                    lines.push(Line::raw(""));
                } else {
                    lines.push(Line::from(Span::styled(
                        "[thinking hidden - ctrl+o or /thinking]",
                        Style::default()
                            .fg(Color::Rgb(105, 110, 130))
                            .add_modifier(Modifier::DIM),
                    )));
                }
            }
            match msg.content.as_str() {
                Some(text) if !text.is_empty() => {
                    let rendered = render_markdown(text);
                    let mut truncated = if rendered.len() > 400 {
                        let mut t = rendered[..400].to_vec();
                        t.push(Line::raw("...[truncated]..."));
                        t
                    } else {
                        rendered
                    };
                    if !truncated.is_empty() {
                        let idx = truncated.iter().position(|l| l.width() > 0).unwrap_or(0);
                        if idx < truncated.len() {
                            let mut spans =
                                vec![Span::styled("◆  ", Style::default().fg(Color::Cyan))];
                            spans.extend(truncated[idx].spans.iter().cloned());
                            truncated[idx] = Line::from(spans);
                        }
                    }
                    lines.extend(truncated);
                }
                _ => {
                    append_value_lines(lines, &msg.content, "", 200);
                }
            }
        }
        MessageRole::System => match msg.content.as_str() {
            Some(text) if !text.is_empty() => {
                lines.extend(render_markdown(text));
            }
            _ => {
                append_value_lines(lines, &msg.content, "", 200);
            }
        },
        MessageRole::Tool => unreachable!(),
    }

    if !msg.tool_calls.is_empty() && tool_details {
        lines.push(Line::raw(""));
        for call in &msg.tool_calls {
            let key_arg = extract_key_arg(&call.name, &call.arguments);
            let label = if key_arg.is_empty() {
                format!("▶ {}", call.name)
            } else {
                format!("▶ {}  {}", call.name, key_arg)
            };
            lines.push(Line::from(vec![Span::styled(
                label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]));
            let pretty = serde_json::from_str::<serde_json::Value>(&call.arguments)
                .ok()
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| call.arguments.clone());
            append_value_lines(lines, &serde_json::Value::String(pretty), "  ", 32);
        }
    }

    let has_text = msg.content.as_str().is_some_and(|t| !t.trim().is_empty());
    let has_reasoning = msg
        .reasoning
        .as_deref()
        .is_some_and(|text| show_reasoning && !text.trim().is_empty());
    if has_text || has_reasoning || msg.role != MessageRole::Assistant {
        lines.push(Line::raw(""));
    }
}
