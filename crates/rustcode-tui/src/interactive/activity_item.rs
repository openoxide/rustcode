use super::{Line, Modifier, Span, Style};

/// A single entry in the activity panel, representing an engine event.
#[derive(Debug, Clone)]
pub(super) enum ActivityItem {
    CommandAccepted {
        name: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        output: String,
    },
    OutputChunk {
        text: String,
    },
    ReasoningChunk {
        text: String,
    },
    Warning {
        message: String,
    },
    Failure {
        message: String,
        /// Full raw error for the details modal (not shown in summary).
        raw_detail: Option<String>,
    },
    /// An LLM call is being retried after a transient failure.
    RetryAttempt {
        attempt: u32,
        max_retries: u32,
        delay_secs: u64,
        reason: String,
    },
    Completed,
}

impl ActivityItem {
    pub(super) fn title(&self) -> String {
        match self {
            ActivityItem::CommandAccepted { name } => format!("command: {name}"),
            ActivityItem::ToolCall { name, .. } => format!("tool: {name}"),
            ActivityItem::ToolResult { name, ok, .. } => format!("tool result: {name} ok={ok}"),
            ActivityItem::OutputChunk { .. } => "assistant".to_string(),
            ActivityItem::ReasoningChunk { .. } => "thinking".to_string(),
            ActivityItem::Warning { .. } => "warning".to_string(),
            ActivityItem::Failure { .. } => "failure".to_string(),
            ActivityItem::RetryAttempt {
                attempt,
                max_retries,
                ..
            } => format!("retry {attempt}/{max_retries}"),
            ActivityItem::Completed => "completed".to_string(),
        }
    }

    pub(super) fn summary(&self) -> String {
        let s = match self {
            ActivityItem::CommandAccepted { name } => {
                // Engine emits format!("{command:?}") — extract the prompt if present
                if let Some(start) = name.find("prompt: \"") {
                    let rest = &name[start + 9..];
                    if let Some(end) = rest.find('"') {
                        let prompt = rest[..end].trim();
                        let short: String = prompt.chars().take(70).collect();
                        return if short.len() < prompt.len() {
                            format!("{short}…")
                        } else {
                            short
                        };
                    }
                }
                // Fallback: show first 70 chars of name
                let short: String = name.chars().take(70).collect();
                if short.len() < name.chars().count() {
                    format!("{short}…")
                } else {
                    short
                }
            }
            ActivityItem::ToolCall {
                name, arguments, ..
            } => {
                // Show the most meaningful argument (path, command, query)
                if let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) {
                    if let Some(p) = args
                        .get("path")
                        .or_else(|| args.get("file_path"))
                        .or_else(|| args.get("target"))
                        .and_then(|v| v.as_str())
                    {
                        return format!("{name}  {p}");
                    }
                    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                        let short: String = cmd.chars().take(55).collect();
                        return if short.len() < cmd.chars().count() {
                            format!("{name}  {short}…")
                        } else {
                            format!("{name}  {short}")
                        };
                    }
                    if let Some(q) = args.get("query").and_then(|v| v.as_str()) {
                        let short: String = q.chars().take(55).collect();
                        return format!("{name}  {short}");
                    }
                }
                name.clone()
            }
            ActivityItem::ToolResult {
                name, ok, output, ..
            } => {
                let indicator = if *ok { "✓" } else { "✗" };
                // Show first meaningful output line
                let snippet = output
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim();
                if snippet.is_empty() {
                    format!("{name}  {indicator}")
                } else {
                    let short: String = snippet.chars().take(60).collect();
                    format!("{name}  {indicator}  {short}")
                }
            }
            ActivityItem::OutputChunk { text } => text.replace('\n', " "),
            ActivityItem::ReasoningChunk { text } => {
                let one_line = text.replace('\n', " ");
                format!("thinking: {one_line}")
            }
            ActivityItem::Warning { message } => message.clone(),
            ActivityItem::Failure { message, .. } => message.clone(),
            ActivityItem::RetryAttempt {
                attempt,
                max_retries,
                delay_secs,
                reason,
            } => {
                format!("retrying in {delay_secs}s ({attempt}/{max_retries}): {reason}")
            }
            ActivityItem::Completed => "done".to_string(),
        };
        if s.chars().count() > 140 {
            format!("{}…", s.chars().take(140).collect::<String>())
        } else {
            s
        }
    }

    pub(super) fn details_lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        out.push(Line::from(vec![Span::styled(
            self.title(),
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        out.push(Line::raw(""));
        match self {
            ActivityItem::CommandAccepted { name } => {
                out.push(Line::raw(format!("name: {name}")));
            }
            ActivityItem::ToolCall {
                id,
                name,
                arguments,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(""));
                out.push(Line::raw("arguments:"));
                let pretty = serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| arguments.clone());
                for line in pretty.lines().take(32) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::ToolResult {
                id,
                name,
                ok,
                output,
            } => {
                out.push(Line::raw(format!("id: {id}")));
                out.push(Line::raw(format!("tool: {name}")));
                out.push(Line::raw(format!("ok: {ok}")));
                out.push(Line::raw(""));
                out.push(Line::raw("output:"));
                let pretty = serde_json::from_str::<serde_json::Value>(output)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or_else(|| output.clone());
                for line in pretty.lines().take(48) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::OutputChunk { text } => {
                for line in text.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::ReasoningChunk { text } => {
                for line in text.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Warning { message } => {
                for line in message.lines().take(64) {
                    out.push(Line::raw(line.to_string()));
                }
            }
            ActivityItem::Failure {
                message,
                raw_detail,
            } => {
                for line in message.lines().take(8) {
                    out.push(Line::raw(line.to_string()));
                }
                if let Some(raw) = raw_detail {
                    out.push(Line::raw(""));
                    out.push(Line::styled(
                        "Raw error:",
                        Style::default().add_modifier(Modifier::DIM),
                    ));
                    for line in raw.lines().take(48) {
                        out.push(Line::raw(line.to_string()));
                    }
                }
            }
            ActivityItem::RetryAttempt {
                attempt,
                max_retries,
                delay_secs,
                reason,
            } => {
                out.push(Line::raw(format!("attempt: {attempt} / {max_retries}")));
                out.push(Line::raw(format!("delay: {delay_secs}s")));
                out.push(Line::raw(format!("reason: {reason}")));
            }
            ActivityItem::Completed => {}
        }

        out
    }
}
