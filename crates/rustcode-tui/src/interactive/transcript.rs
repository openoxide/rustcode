use std::collections::HashMap;

use super::{
    markdown::render_markdown, push_toast, ActivityItem, AppState, ChatState, Color, Duration,
    FindState, Line, MessageRole, Modifier, Span, StoredMessage, Style, SystemTime, ToastVariant,
};

mod append;
mod helpers;
mod plan_todo;
mod render;

use append::{append_message_lines, append_tool_message_lines};
use helpers::{
    extract_key_arg, format_elapsed, is_editing_tool, line_plain, parse_tool_payload,
    sanitize_output_line,
};
use render::{render_live_activity_lines, render_live_activity_summary, render_tool_output};

fn emit_combined_tool_summary(
    lines: &mut Vec<Line<'static>>,
    pending: &[&StoredMessage],
    tool_details: bool,
    tool_args_map: &HashMap<String, String>,
) {
    if pending.is_empty() {
        return;
    }
    if tool_details {
        for msg in pending {
            append_tool_message_lines(lines, msg, true, tool_args_map);
        }
    } else {
        let latest = pending.last().copied();
        let latest_name = latest
            .and_then(|m| m.tool_name.as_deref())
            .unwrap_or("tool");
        let latest_key_arg = latest
            .and_then(|m| m.tool_call_id.as_ref())
            .and_then(|id| tool_args_map.get(id))
            .map(|args| extract_key_arg(latest_name, args))
            .unwrap_or_default();
        let latest_label = if latest_key_arg.is_empty() {
            latest_name.to_string()
        } else {
            format!("{latest_name}({latest_key_arg})")
        };
        let batch_total = pending.len();
        let prior_calls = batch_total.saturating_sub(1);
        let count_part = if prior_calls > 0 {
            format!(
                "  +{prior_calls} more tool use{}",
                if prior_calls == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        let total_failures = pending
            .iter()
            .filter(|msg| {
                parse_tool_payload(msg.content.as_str().unwrap_or("")).is_some_and(|(ok, _, _)| !ok)
            })
            .count();
        let fail_part = if total_failures > 0 {
            format!("  {total_failures} \u{2717}")
        } else {
            String::new()
        };
        lines.push(Line::from(Span::styled(
            format!("  \u{25b6} {latest_label}{count_part}{fail_part}  (ctrl+o to expand)"),
            Style::default().fg(Color::Rgb(160, 165, 180)),
        )));
        // Show diffs for ALL editing tools in the batch (not just the latest).
        for msg in pending {
            let msg_name = msg.tool_name.as_deref().unwrap_or("tool");
            if is_editing_tool(msg_name) {
                let content = msg.content.as_str().unwrap_or("");
                if let Some((ok, _truncated, output)) = parse_tool_payload(content) {
                    if ok && output.contains("\n@@diff\n") {
                        render_tool_output(lines, msg_name, &output, true, ok);
                    }
                }
            }
        }
        lines.push(Line::raw(""));
    }
}

/// Emit diff output for every editing tool in `pending` without any summary
/// header.  Used for earlier tool batches in collapsed mode so file changes
/// are always visible in the transcript regardless of expand/collapse state.
fn emit_editing_diffs(lines: &mut Vec<Line<'static>>, pending: &[&StoredMessage]) {
    for msg in pending {
        let name = msg.tool_name.as_deref().unwrap_or("tool");
        if is_editing_tool(name) {
            let content = msg.content.as_str().unwrap_or("");
            if let Some((ok, _, output)) = parse_tool_payload(content) {
                if ok && output.contains("\n@@diff\n") {
                    render_tool_output(lines, name, &output, true, ok);
                }
            }
        }
    }
}

pub(super) fn build_transcript_lines(chat: &ChatState) -> Vec<Line<'static>> {
    let mut tool_args_map = HashMap::new();
    for msg in &chat.messages {
        if msg.role == MessageRole::Assistant {
            for call in &msg.tool_calls {
                tool_args_map.insert(call.id.clone(), call.arguments.clone());
            }
        }
    }

    let mut lines = Vec::new();

    let last_tool_idx = chat
        .messages
        .iter()
        .rposition(|m| m.role == MessageRole::Tool);

    let mut pending_tools: Vec<&StoredMessage> = Vec::new();
    let mut pending_includes_last = false;
    let mut collapsed_summary_shown = false;

    for (msg_idx, msg) in chat.messages.iter().enumerate() {
        if msg.role == MessageRole::System {
            continue;
        }
        if msg.role == MessageRole::Tool {
            pending_tools.push(msg);
            if Some(msg_idx) == last_tool_idx {
                pending_includes_last = true;
            }
            continue;
        }
        if !pending_tools.is_empty() {
            if chat.tool_details {
                for m in &pending_tools {
                    append_tool_message_lines(&mut lines, m, chat.output_details, &tool_args_map);
                }
            } else if pending_includes_last && !collapsed_summary_shown {
                emit_combined_tool_summary(&mut lines, &pending_tools, false, &tool_args_map);
                collapsed_summary_shown = true;
            } else {
                emit_editing_diffs(&mut lines, &pending_tools);
            }
            pending_tools.clear();
            pending_includes_last = false;
        }
        append_message_lines(
            &mut lines,
            msg,
            chat.tool_details,
            chat.output_details,
            &tool_args_map,
            chat.show_reasoning,
        );
    }
    // Handle trailing Tool messages (conversation ending with a tool result).
    if !pending_tools.is_empty() {
        if chat.tool_details {
            for m in &pending_tools {
                append_tool_message_lines(&mut lines, m, chat.output_details, &tool_args_map);
            }
        } else if !collapsed_summary_shown {
            emit_combined_tool_summary(&mut lines, &pending_tools, false, &tool_args_map);
        } else {
            emit_editing_diffs(&mut lines, &pending_tools);
        }
    }
    // Show the in-flight user prompt immediately (before backend confirms it)
    if let Some(pending) = &chat.pending_prompt {
        if !pending.trim().is_empty() {
            let bg = Style::default().bg(Color::Rgb(25, 45, 80));
            let mut rendered = render_markdown(pending.as_str());
            if !rendered.is_empty() {
                let idx = rendered.iter().position(|l| l.width() > 0).unwrap_or(0);
                if idx < rendered.len() {
                    let mut spans = vec![Span::styled(
                        "▶  ",
                        Style::default().fg(Color::Rgb(120, 200, 200)),
                    )];
                    spans.extend(rendered[idx].spans.iter().cloned());
                    rendered[idx] = Line::from(spans);
                }
            }
            lines.extend(rendered.into_iter().map(|l| l.style(bg)));
            lines.push(Line::raw(""));
        }
    }
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis();

    if chat.show_reasoning && !chat.live_reasoning.trim().is_empty() {
        let rendered = render_markdown(chat.live_reasoning.as_str());
        let mut truncated = if rendered.len() > 400 {
            let mut t = rendered[..400].to_vec();
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
    }

    if !chat.live_assistant.trim().is_empty() {
        let rendered = render_markdown(chat.live_assistant.as_str());
        let mut truncated = if rendered.len() > 400 {
            let mut t = rendered[..400].to_vec();
            t.push(Line::raw("...[streaming truncated]..."));
            t
        } else {
            rendered
        };
        if !truncated.is_empty() {
            let idx = truncated.iter().position(|l| l.width() > 0).unwrap_or(0);
            if idx < truncated.len() {
                let mut spans = vec![Span::styled("◆  ", Style::default().fg(Color::Cyan))];
                spans.extend(truncated[idx].spans.iter().cloned());
                truncated[idx] = Line::from(spans);
            }
        }
        lines.extend(truncated);
        lines.push(Line::raw(""));
    }
    // ── Live plan/todo widgets ──────────────────────────────────────────
    plan_todo::render_plan_lines(&mut lines, chat);
    plan_todo::render_todo_lines(&mut lines, chat);

    if chat.running.is_some() {
        let has_tool_calls = chat
            .activity
            .iter()
            .any(|item| matches!(item, ActivityItem::ToolCall { .. }));

        if has_tool_calls {
            if chat.tool_details {
                render_live_activity_lines(&mut lines, &chat.activity, ms);
                // In expanded mode keep elapsed as a trailing dim line.
                if let Some(started) = chat.run_started_at {
                    lines.push(Line::from(Span::styled(
                        format!("   {}", format_elapsed(started.elapsed())),
                        Style::default()
                            .fg(Color::Rgb(50, 55, 65))
                            .add_modifier(Modifier::DIM),
                    )));
                }
            } else {
                render_live_activity_summary(&mut lines, &chat.activity, ms);
                // In collapsed mode inline elapsed into the summary line so it
                // doesn't float as a disconnected standalone line.
                if let Some(started) = chat.run_started_at {
                    if let Some(last) = lines.last_mut() {
                        last.spans.push(Span::styled(
                            format!("  {}", format_elapsed(started.elapsed())),
                            Style::default()
                                .fg(Color::Rgb(70, 75, 90))
                                .add_modifier(Modifier::DIM),
                        ));
                    }
                }
            }
        } else if chat.live_assistant.trim().is_empty() {
            const FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
            let frame = FRAMES[(ms / 150 % 4) as usize];
            let thinking_hint = if chat.show_reasoning {
                "thinking…"
            } else {
                "thinking… [ctrl+o to show]"
            };
            let mut thinking_spans = vec![
                Span::styled("◆  ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{frame} {thinking_hint}"),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::DIM),
                ),
            ];
            if let Some(started) = chat.run_started_at {
                thinking_spans.push(Span::styled(
                    format!("  {}", format_elapsed(started.elapsed())),
                    Style::default()
                        .fg(Color::Rgb(55, 60, 75))
                        .add_modifier(Modifier::DIM),
                ));
            }
            lines.push(Line::from(thinking_spans));
        }
    }

    if let Some(elapsed) = chat.last_run_elapsed {
        lines.push(Line::from(Span::styled(
            format!("* done in {}", format_elapsed(elapsed)),
            Style::default()
                .fg(Color::Rgb(90, 95, 110))
                .add_modifier(Modifier::DIM),
        )));
        lines.push(Line::raw(""));
    }

    if chat.running.is_none() {
        if let Some(error) = chat.activity.iter().rev().find_map(|item| match item {
            ActivityItem::Failure { message, .. } => Some(message.as_str()),
            _ => None,
        }) {
            let clean = sanitize_output_line(error)
                .chars()
                .take(200)
                .collect::<String>();
            if !clean.trim().is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("✗ {clean}"),
                    Style::default().fg(Color::Rgb(205, 95, 95)),
                )));
                lines.push(Line::raw(""));
            }
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::raw(""));
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
    let max_scroll = total_lines.saturating_sub(viewport_h);
    let scroll_top = line_idx.saturating_sub(2).min(max_scroll);
    chat.scroll = max_scroll.saturating_sub(scroll_top);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_key_arg_for_apply_patch_prefers_target_file() {
        let args = serde_json::json!({
            "patch_text": "*** Begin Patch\n*** Update File: crates/rustcode-memories/src/phase1.rs\n@@\n-old\n+new\n*** End Patch\n"
        })
        .to_string();
        let key = extract_key_arg("apply_patch", &args);
        assert_eq!(key, "crates/rustcode-memories/src/phase1.rs");
    }

    #[test]
    fn extract_patch_target_supports_unified_and_codex_headers() {
        use helpers::extract_patch_target;
        let unified = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let codex = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch\n";
        assert_eq!(
            extract_patch_target(unified),
            Some("src/main.rs".to_string())
        );
        assert_eq!(extract_patch_target(codex), Some("src/lib.rs".to_string()));
    }

    #[test]
    fn parse_hunk_header_extracts_old_and_new_starts() {
        use helpers::parse_hunk_header;
        let parsed = parse_hunk_header("@@ -284,7 +284,6 @@").expect("hunk should parse");
        assert_eq!(parsed, (283, 283));
    }
}
