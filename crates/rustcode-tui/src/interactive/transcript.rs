use std::collections::HashMap;

use super::{
    markdown::render_markdown, push_toast,
    syntax_highlight::{highlight_code_line, HighlightState},
    AppState, ChatState, Color, Duration, FindState, Line, MessageRole, Modifier, Span,
    StoredMessage, Style, SystemTime, ToastVariant,
};

fn append_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &StoredMessage,
    tool_details: bool,
    tool_args_map: &HashMap<String, String>,
) {
    // Tool result messages are rendered inline without a role header — the ✓/✗
    // indicator and tool name are self-descriptive.
    if msg.role == MessageRole::Tool {
        append_tool_message_lines(lines, msg, tool_details, tool_args_map);
        // No trailing blank line — tool results attach visually to the assistant turn
        return;
    }

    let role = match msg.role {
        MessageRole::System => "System",
        MessageRole::User => "You",
        MessageRole::Assistant => "Assistant",
        MessageRole::Tool => unreachable!(),
    };

    let role_style = match msg.role {
        MessageRole::System => Style::default().add_modifier(Modifier::DIM),
        MessageRole::User => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        MessageRole::Assistant => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        MessageRole::Tool => unreachable!(),
    };

    lines.push(Line::from(vec![Span::styled(role, role_style)]));

    {
        // Use markdown rendering for user/assistant/system messages
        match msg.content.as_str() {
            Some(text) if !text.is_empty() => {
                let rendered = render_markdown(text);
                let truncated = if rendered.len() > 400 {
                    let mut t = rendered[..400].to_vec();
                    t.push(Line::raw("...[truncated]..."));
                    t
                } else {
                    rendered
                };
                lines.extend(truncated);
            }
            _ => {
                append_value_lines(lines, &msg.content, "", 200);
            }
        }
        if !msg.tool_calls.is_empty() {
            if tool_details {
                lines.push(Line::raw(""));
                // Expanded: show each ▶ tool_name with key arg and optional JSON
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
            // Collapsed: nothing shown — the ✓/✗ result follows.
        }
    }

    // Only add a trailing blank spacer if there was visible text content.
    // Empty assistant messages (with tool_calls but no text) should flow
    // directly into their tool result lines without a gap.
    let has_text = msg.content.as_str().is_some_and(|t| !t.trim().is_empty());
    if has_text || msg.role != MessageRole::Assistant {
        lines.push(Line::raw(""));
    }
}

fn append_tool_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &StoredMessage,
    tool_details: bool,
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

        // Build the header: "  ✓ tool  context"
        let mut header_spans = vec![Span::styled(
            format!("  {indicator} {name}"),
            indicator_style,
        )];

        // Extract tool arguments from the tool_call correlation map.
        let args_json = msg
            .tool_call_id
            .as_ref()
            .and_then(|id| tool_args_map.get(id))
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

        // Append contextual info based on tool type.
        append_tool_context(&mut header_spans, name, args_json.as_ref());

        lines.push(Line::from(header_spans));

        if tool_details {
            render_tool_output(lines, &output, true, ok);
        } else {
            render_tool_output(lines, &output, false, ok);
        }
        return;
    }

    // Fallback for non-standard tool payloads
    lines.push(Line::from(vec![Span::styled(
        format!("  ▶ {name}"),
        Style::default().fg(Color::Cyan),
    )]));
    if !content_str.is_empty() {
        append_value_lines(lines, &msg.content, "  ", 20);
    }
}

/// Render tool output with GitHub-patch-style diff rendering.
///
/// Diff output gets a file header, coloured stat, and left-border pipe (`│`)
/// on every diff line for a clean patch-like appearance.
///
/// Plain output: collapsed shows first non-empty line; expanded shows up to 20 lines.
fn render_tool_output(lines: &mut Vec<Line<'static>>, output: &str, expanded: bool, ok: bool) {
    const DIFF_MARKER: &str = "\n@@diff\n";
    if let Some(diff_pos) = output.find(DIFF_MARKER) {
        let summary = output[..diff_pos].trim();
        let diff_body = &output[diff_pos + DIFF_MARKER.len()..];

        // Parse summary into description + stat parts.
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

        // Extract filename from description (last path token) — full path with ~.
        let filename = extract_filename(&desc_line);

        // File header: "  ┌─ ~/path/file.py  (+N -M)"
        let mut header_spans: Vec<Span<'static>> =
            vec![Span::styled("  ┌─ ", Style::default().fg(Color::DarkGray))];
        header_spans.push(Span::styled(
            filename,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        if !stat_line.is_empty() {
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled("(", Style::default().fg(Color::DarkGray)));
            for (i, word) in stat_line.split_whitespace().enumerate() {
                if i > 0 {
                    header_spans.push(Span::raw(" "));
                }
                let style = if word.starts_with('+') {
                    Style::default().fg(Color::Green)
                } else if word.starts_with('-') {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                header_spans.push(Span::styled(word.to_string(), style));
            }
            header_spans.push(Span::styled(")", Style::default().fg(Color::DarkGray)));
        }
        lines.push(Line::from(header_spans));

        if !expanded {
            // Collapsed: just the header is enough — the user already saw
            // the diff in the approval preview.
        } else {
            // Expanded: show full diff body with syntax highlighting.
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
                let (gutter, content_style, pipe_color) =
                    if raw.starts_with("+++") || raw.starts_with("---") {
                        (
                            "    ".to_string(),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                            Color::DarkGray,
                        )
                    } else if raw.starts_with("@@") {
                        if let Some(range) = parse_hunk_header(raw) {
                            add_no = range.0;
                            del_no = range.1;
                        }
                        (
                            "    ".to_string(),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                            Color::DarkGray,
                        )
                    } else if raw.starts_with('+') {
                        add_no += 1;
                        (
                            format!("{:>3} ", add_no),
                            Style::default()
                                .fg(Color::Rgb(80, 200, 80))
                                .bg(Color::Rgb(3, 40, 0)),
                            Color::Rgb(80, 200, 80),
                        )
                    } else if raw.starts_with('-') {
                        del_no += 1;
                        (
                            format!("{:>3} ", del_no),
                            Style::default()
                                .fg(Color::Rgb(200, 80, 80))
                                .bg(Color::Rgb(61, 1, 0)),
                            Color::Rgb(200, 80, 80),
                        )
                    } else {
                        add_no += 1;
                        del_no += 1;
                        (
                            format!("{:>3} ", add_no),
                            Style::default().add_modifier(Modifier::DIM),
                            Color::DarkGray,
                        )
                    };

                let code_text = raw.get(1..).unwrap_or(raw);
                let is_diff_line = raw.starts_with('+') || raw.starts_with('-');
                let sign = if raw.starts_with('+') {
                    "+"
                } else if raw.starts_with('-') {
                    "-"
                } else {
                    ""
                };

                if is_diff_line {
                    let mut line_spans = vec![
                        Span::styled("  │ ", Style::default().fg(pipe_color)),
                        Span::styled(
                            gutter,
                            Style::default().fg(pipe_color),
                        ),
                        Span::styled(sign.to_string(), Style::default().fg(pipe_color)),
                    ];
                    if let Some(ref mut state) = hl {
                        let hl_spans = highlight_code_line(code_text, state);
                        if !hl_spans.is_empty() {
                            line_spans.extend(hl_spans);
                        } else {
                            line_spans.push(Span::styled(
                                code_text.to_string(),
                                content_style,
                            ));
                        }
                    } else {
                        line_spans.push(Span::styled(
                            code_text.to_string(),
                            content_style,
                        ));
                    }
                    let bg = if raw.starts_with('+') {
                        Style::default().bg(Color::Rgb(3, 40, 0))
                    } else {
                        Style::default().bg(Color::Rgb(61, 1, 0))
                    };
                    lines.push(Line::from(line_spans).style(bg));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Style::default().fg(pipe_color)),
                        Span::styled(gutter, Style::default().add_modifier(Modifier::DIM)),
                        Span::styled(raw.to_string(), content_style),
                    ]));
                }
            }
        }

        // Bottom border.
        lines.push(Line::from(Span::styled(
            "  └─",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Plain (non-diff) output — collapse long output, style errors.
        // Apply tilde_path to shorten absolute paths in output.
        let output_lines: Vec<String> = output
            .lines()
            .map(|l| tilde_path_in_line(l))
            .collect();
        let total = output_lines.len();
        let text_style = if !ok {
            Style::default().fg(Color::Rgb(200, 80, 80)) // Red for errors
        } else {
            Style::default().add_modifier(Modifier::DIM) // Dimmed for success
        };

        if expanded {
            // Expanded: show up to 50 lines.
            for (idx, line) in output_lines.iter().enumerate() {
                if idx >= 50 {
                    lines.push(Line::from(Span::styled(
                        "  …[truncated]…",
                        Style::default().add_modifier(Modifier::DIM),
                    )));
                    break;
                }
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    text_style,
                )));
            }
        } else if total > 6 {
            // Collapsed, long output: show first 3 + last 3 with a hint.
            for line in output_lines.iter().take(3) {
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    text_style,
                )));
            }
            lines.push(Line::from(Span::styled(
                format!("  ··· ({} more lines — press > to expand)", total - 6),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
            for line in output_lines.iter().skip(total - 3) {
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    text_style,
                )));
            }
        } else if total > 0 {
            // Short output: show all lines.
            for line in &output_lines {
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    text_style,
                )));
            }
        }
        // else: empty output — the ✓/✗ header is enough.
    }
}

/// Extract the most meaningful short argument from a tool call for inline display.
fn extract_key_arg(tool: &str, arguments: &str) -> String {
    let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return String::new();
    };
    // Path-like tools
    if let Some(p) = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .or_else(|| args.get("target"))
        .and_then(|v| v.as_str())
    {
        let short: String = p.chars().take(60).collect();
        return if short.len() < p.chars().count() {
            format!("{short}…")
        } else {
            short
        };
    }
    // Command-like tools
    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
        let short: String = cmd.chars().take(55).collect();
        return if short.len() < cmd.chars().count() {
            format!("{short}…")
        } else {
            short
        };
    }
    // Query/search tools
    if let Some(q) = args
        .get("query")
        .or_else(|| args.get("pattern"))
        .and_then(|v| v.as_str())
    {
        let short: String = q.chars().take(55).collect();
        return short;
    }
    // Fallback: first string value found
    if let Some(obj) = args.as_object() {
        for (key, val) in obj.iter() {
            if key == "contents" || key == "content" || key == "code" {
                continue; // Skip large bodies
            }
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    let short: String = s.chars().take(55).collect();
                    return short;
                }
            }
        }
    }
    let _ = tool; // suppress unused warning
    String::new()
}

fn parse_tool_payload(content: &str) -> Option<(bool, bool, String)> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let ok = value.get("ok")?.as_bool()?;
    let truncated = value.get("truncated")?.as_bool().unwrap_or(false);
    let output = value
        .get("output")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    Some((ok, truncated, output))
}

fn append_value_lines(
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

fn append_text_lines(lines: &mut Vec<Line<'static>>, text: &str, prefix: &str, max_lines: usize) {
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

fn line_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

pub(super) fn compute_find_matches(lines: &[Line<'static>], query: &str) -> Vec<usize> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let hay = line_plain(line).to_ascii_lowercase();
        if hay.contains(&needle) {
            out.push(idx);
        }
    }
    out
}

pub(super) fn build_transcript_lines(chat: &ChatState) -> Vec<Line<'static>> {
    // Build tool_call_id → arguments map from Assistant messages so tool results
    // can display context (e.g., the command for bash, the pattern for glob).
    let mut tool_args_map = HashMap::new();
    for msg in &chat.messages {
        if msg.role == MessageRole::Assistant {
            for call in &msg.tool_calls {
                tool_args_map.insert(call.id.clone(), call.arguments.clone());
            }
        }
    }

    let mut lines = Vec::new();
    for msg in &chat.messages {
        // Skip system messages — internal instructions are never shown to users
        if msg.role == MessageRole::System {
            continue;
        }
        append_message_lines(&mut lines, msg, chat.tool_details, &tool_args_map);
    }
    // Show the in-flight user prompt immediately (before backend confirms it)
    if let Some(pending) = &chat.pending_prompt {
        if !pending.trim().is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "You",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]));
            let rendered = render_markdown(pending.as_str());
            lines.extend(rendered);
            lines.push(Line::raw(""));
        }
    }
    if !chat.live_assistant.trim().is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Assistant (streaming)",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]));
        let rendered = render_markdown(chat.live_assistant.as_str());
        let truncated = if rendered.len() > 400 {
            let mut t = rendered[..400].to_vec();
            t.push(Line::raw("...[streaming truncated]..."));
            t
        } else {
            rendered
        };
        lines.extend(truncated);
        lines.push(Line::raw(""));
    } else if chat.running.is_some() {
        // No streaming text yet — show animated thinking indicator.
        // Quarter-circle spinner: ◐◓◑◒ (4 frames × 150 ms each)
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();
        const FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
        let frame = FRAMES[(ms / 150 % 4) as usize];
        lines.push(Line::from(vec![
            Span::styled(
                "Assistant",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{frame} thinking\u{2026}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    }
    lines
}

pub(super) fn apply_find_highlight(
    lines: Vec<Line<'static>>,
    find: Option<&FindState>,
) -> Vec<Line<'static>> {
    let Some(find) = find else {
        return lines;
    };
    if find.query.trim().is_empty() || find.matches.is_empty() {
        return lines;
    }

    let mut out = Vec::with_capacity(lines.len());
    for (idx, line) in lines.into_iter().enumerate() {
        if !find.matches.contains(&idx) {
            out.push(line);
            continue;
        }
        let mut spans = Vec::new();
        for span in line.spans {
            let mut style = span.style.add_modifier(Modifier::REVERSED);
            if find
                .matches
                .get(find.current)
                .is_some_and(|cur| *cur == idx)
            {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(span.content.into_owned(), style));
        }
        out.push(Line::from(spans));
    }
    out
}

fn jump_transcript_to_line(
    chat: &mut ChatState,
    line_idx: usize,
    viewport_h: u16,
    total_lines: usize,
) {
    let viewport_h = viewport_h.saturating_sub(2).max(1) as usize;
    if total_lines <= viewport_h {
        chat.scroll = 0;
        return;
    }
    let max_scroll = total_lines.saturating_sub(viewport_h) as u16;
    let scroll_top = line_idx.saturating_sub(2).min(max_scroll as usize) as u16;
    chat.scroll = max_scroll.saturating_sub(scroll_top);
}

pub(super) fn set_find(
    state: &mut AppState,
    chat: &mut ChatState,
    query: String,
    jump: bool,
    viewport_h: u16,
) {
    let trimmed = query.trim().to_string();
    if trimmed.is_empty() {
        chat.find = None;
        return;
    }

    let transcript = build_transcript_lines(chat);
    let matches = compute_find_matches(&transcript, &trimmed);
    let current = 0usize;
    chat.find = Some(FindState {
        query: trimmed,
        matches,
        current,
    });

    if jump {
        let (line_idx, current, total) = chat
            .find
            .as_ref()
            .and_then(|find| {
                find.matches
                    .get(find.current)
                    .copied()
                    .map(|idx| (idx, find.current, find.matches.len()))
            })
            .unwrap_or((usize::MAX, 0, 0));

        if line_idx == usize::MAX {
            push_toast(
                state,
                ToastVariant::Warning,
                "no matches",
                Duration::from_secs(2),
            );
            return;
        }
        jump_transcript_to_line(chat, line_idx, viewport_h, transcript.len());
        push_toast(
            state,
            ToastVariant::Info,
            format!("match {}/{}", current + 1, total),
            Duration::from_secs(2),
        );
    }
}

pub(super) fn find_next(state: &mut AppState, chat: &mut ChatState, viewport_h: u16) {
    let Some(find) = chat.find.as_ref() else {
        push_toast(
            state,
            ToastVariant::Warning,
            "no active search",
            Duration::from_secs(2),
        );
        return;
    };
    let transcript = build_transcript_lines(chat);
    let matches = compute_find_matches(&transcript, &find.query);
    if matches.is_empty() {
        push_toast(
            state,
            ToastVariant::Warning,
            "no matches",
            Duration::from_secs(2),
        );
        if let Some(find) = &mut chat.find {
            find.matches.clear();
            find.current = 0;
        }
        return;
    }

    let base = find.current.min(matches.len().saturating_sub(1));
    let next_current = (base + 1) % matches.len();
    let line_idx = matches[next_current];
    jump_transcript_to_line(chat, line_idx, viewport_h, transcript.len());
    if let Some(find) = &mut chat.find {
        find.matches = matches.clone();
        find.current = next_current;
    }
    push_toast(
        state,
        ToastVariant::Info,
        format!("match {}/{}", next_current + 1, matches.len()),
        Duration::from_secs(2),
    );
}

/// Returns `true` if `s` looks like a diff stat line, e.g. `"+22 -18"`.
///
/// All whitespace-separated tokens must start with `+` or `-` followed only by
/// ASCII digits. This prevents false matches on code lines like `result = a - 5`.
fn is_stat_line(s: &str) -> bool {
    let parts: Vec<&str> = s.split_whitespace().collect();
    !parts.is_empty()
        && parts.len() <= 4
        && parts.iter().all(|p| {
            (p.starts_with('+') || p.starts_with('-')) && p[1..].chars().all(|c| c.is_ascii_digit())
        })
}

/// Parse a unified diff hunk header like `@@ -10,5 +12,7 @@` and return
/// `(new_start, old_start)` so callers can reset their line counters.
fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    // Format: @@ -OLD_START[,OLD_LEN] +NEW_START[,NEW_LEN] @@
    let trimmed = header.trim_start_matches('@').trim();
    let mut old_start = 0usize;
    let mut new_start = 0usize;
    for token in trimmed.split_whitespace() {
        if let Some(rest) = token.strip_prefix('-') {
            old_start = rest
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(rest) = token.strip_prefix('+') {
            new_start = rest
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }
    if old_start > 0 || new_start > 0 {
        // Return (add_counter, del_counter) as zero-based so the first ++
        // will produce the correct 1-based number.
        Some((new_start.saturating_sub(1), old_start.saturating_sub(1)))
    } else {
        None
    }
}

/// Extract a file path from a tool description line like
/// `"edit applied (1 replacements) to /very/long/path/file.py"`.
///
/// Returns the full path with `~` replacing `$HOME` for readability.
/// Falls back to the description itself.
fn extract_filename(desc: &str) -> String {
    for token in desc.split_whitespace().rev() {
        if token.contains('/') {
            return tilde_path(token);
        }
    }
    desc.to_string()
}

/// Replace `$HOME` prefix with `~` for compact path display.
fn tilde_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = path.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// Replace all occurrences of `$HOME` in a line with `~`.
fn tilde_path_in_line(line: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if line.contains(&home) {
            return line.replace(&home, "~");
        }
    }
    line.to_string()
}

/// Append contextual spans to the tool result header based on tool type and args.
///
/// For example: `✓ bash  $ cargo build`, `✓ glob  *.py`, `✓ websearch  "rust TUI"`.
fn append_tool_context(
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
        // Command tools: show `$ command` with bash syntax highlighting.
        "bash" | "exec" | "pty_exec" => {
            if let Some(cmd) = get(&["command", "cmd"]) {
                let (s, trunc) = short(&cmd, 60);
                spans.push(Span::styled("  $ ", Style::default().fg(Color::DarkGray)));
                let mut hl_bash = HighlightState::new("sh");
                if let Some(ref mut state) = hl_bash {
                    let hl_spans = highlight_code_line(&s, state);
                    if !hl_spans.is_empty() {
                        spans.extend(hl_spans);
                    } else {
                        spans.push(Span::styled(s, Style::default().fg(Color::DarkGray)));
                    }
                } else {
                    spans.push(Span::styled(s, Style::default().fg(Color::DarkGray)));
                }
                if trunc {
                    spans.push(Span::styled("…", Style::default().fg(Color::DarkGray)));
                }
            }
        }
        // Search tools: show the pattern/query.
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
        // File tools: show the path.
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
        // Web tools: show query or URL.
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
        // Plan/task: show the title.
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
        // write/edit/multiedit/apply_patch — context shown via diff header, skip.
        // batch/question/todowrite/snapshot_*/worktree_* — no concise context.
        _ => {}
    }
}

pub(super) fn find_prev(state: &mut AppState, chat: &mut ChatState, viewport_h: u16) {
    let Some(find) = chat.find.as_ref() else {
        push_toast(
            state,
            ToastVariant::Warning,
            "no active search",
            Duration::from_secs(2),
        );
        return;
    };
    let transcript = build_transcript_lines(chat);
    let matches = compute_find_matches(&transcript, &find.query);
    if matches.is_empty() {
        push_toast(
            state,
            ToastVariant::Warning,
            "no matches",
            Duration::from_secs(2),
        );
        if let Some(find) = &mut chat.find {
            find.matches.clear();
            find.current = 0;
        }
        return;
    }

    let base = find.current.min(matches.len().saturating_sub(1));
    let prev_current = if base == 0 {
        matches.len() - 1
    } else {
        base - 1
    };
    let line_idx = matches[prev_current];
    jump_transcript_to_line(chat, line_idx, viewport_h, transcript.len());
    if let Some(find) = &mut chat.find {
        find.matches = matches.clone();
        find.current = prev_current;
    }
    push_toast(
        state,
        ToastVariant::Info,
        format!("match {}/{}", prev_current + 1, matches.len()),
        Duration::from_secs(2),
    );
}
