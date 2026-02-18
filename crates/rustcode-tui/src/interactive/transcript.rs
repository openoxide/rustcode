use super::*;

fn append_message_lines(lines: &mut Vec<Line<'static>>, msg: &StoredMessage, tool_details: bool) {
    let role = match msg.role {
        MessageRole::System => "System",
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
        MessageRole::Tool => "Tool",
    };

    let role_style = match msg.role {
        MessageRole::System => Style::default().add_modifier(Modifier::DIM),
        MessageRole::User => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        MessageRole::Assistant => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        MessageRole::Tool => Style::default().add_modifier(Modifier::BOLD),
    };

    lines.push(Line::from(vec![Span::styled(role, role_style)]));

    match msg.role {
        MessageRole::Tool => append_tool_message_lines(lines, msg, tool_details),
        _ => {
            append_value_lines(lines, &msg.content, "", 200);
            if !msg.tool_calls.is_empty() {
                lines.push(Line::raw(""));
                for call in &msg.tool_calls {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("Tool: {}", call.name),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(format!("  id={} ", call.id)),
                    ]));

                    if tool_details {
                        let pretty = serde_json::from_str::<serde_json::Value>(&call.arguments)
                            .ok()
                            .and_then(|value| serde_json::to_string_pretty(&value).ok())
                            .unwrap_or_else(|| call.arguments.clone());
                        append_value_lines(lines, &serde_json::Value::String(pretty), "  ", 64);
                    }
                }
            }
        }
    }

    lines.push(Line::raw(""));
}

fn append_tool_message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &StoredMessage,
    tool_details: bool,
) {
    let name = msg.tool_name.as_deref().unwrap_or("tool");
    let call_id = msg.tool_call_id.as_deref().unwrap_or("-");
    let content_str = msg.content.as_str().unwrap_or("");

    if let Some((ok, truncated, output)) = parse_tool_payload(content_str) {
        lines.push(Line::raw(format!(
            "Tool: {name}  id={call_id}  ok={ok}  truncated={truncated}"
        )));
        if tool_details {
            lines.push(Line::raw(""));
            lines.push(Line::raw("Output:"));
            append_value_lines(lines, &serde_json::Value::String(output), "  ", 200);
        } else {
            let snippet = output.lines().next().unwrap_or("").trim().to_string();
            if !snippet.is_empty() {
                lines.push(Line::raw(format!("  {snippet}")));
            }
        }
        return;
    }

    if !content_str.is_empty() {
        lines.push(Line::raw(format!("Tool: {name}  id={call_id}")));
        append_value_lines(lines, &msg.content, "  ", 200);
    } else {
        lines.push(Line::raw(format!("Tool: {name}  id={call_id}")));
    }
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
            append_text_lines(lines, text.as_str(), prefix, max_lines)
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
    let mut lines = Vec::new();
    for msg in &chat.messages {
        append_message_lines(&mut lines, msg, chat.tool_details);
    }
    if !chat.live_assistant.trim().is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Assistant (streaming)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]));
        append_text_lines(&mut lines, chat.live_assistant.as_str(), "", 200);
        lines.push(Line::raw(""));
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
