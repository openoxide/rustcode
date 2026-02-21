//! Pure helper utilities for transcript rendering: path utilities, stat/hunk parsers,
//! output sanitisation, and JSON payload extraction.

use ratatui::text::Line;

/// Returns `true` if `s` looks like a diff stat line, e.g. `"+22 -18"`.
///
/// All whitespace-separated tokens must start with `+` or `-` followed only by
/// ASCII digits. This prevents false matches on code lines like `result = a - 5`.
pub(super) fn is_stat_line(s: &str) -> bool {
    let parts: Vec<&str> = s.split_whitespace().collect();
    !parts.is_empty()
        && parts.len() <= 4
        && parts.iter().all(|p| {
            (p.starts_with('+') || p.starts_with('-')) && p[1..].chars().all(|c| c.is_ascii_digit())
        })
}

/// Parse a unified diff hunk header like `@@ -10,5 +12,7 @@` and return
/// `(new_start, old_start)` so callers can reset their line counters.
pub(super) fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
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
        Some((new_start.saturating_sub(1), old_start.saturating_sub(1)))
    } else {
        None
    }
}

pub(super) fn extract_patch_target(patch_text: &str) -> Option<String> {
    for line in patch_text.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let trimmed = path.trim().trim_start_matches("b/");
            if !trimmed.is_empty() && trimmed != "/dev/null" {
                return Some(trimmed.to_string());
            }
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extract a file path from a tool description line, replacing `$HOME` with `~`.
pub(super) fn extract_filename(desc: &str) -> String {
    if let Some(path) = desc.strip_prefix("file:") {
        let path = path.trim();
        if !path.is_empty() {
            return tilde_path(path);
        }
    }
    for token in desc.split_whitespace().rev() {
        if token.contains('/') {
            return tilde_path(token);
        }
    }
    desc.to_string()
}

/// Replace `$HOME` prefix with `~` for compact path display.
pub(super) fn tilde_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = path.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// Replace all occurrences of `$HOME` in a line with `~`.
pub(super) fn tilde_path_in_line(line: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if line.contains(&home) {
            return line.replace(&home, "~");
        }
    }
    line.to_string()
}

/// Strip ANSI escape sequences and ASCII control characters from tool output.
pub(super) fn sanitize_output_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for nc in chars.by_ref() {
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c.is_control() && c != '\t' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Format a `Duration` as a human-readable string like `"30s"`, `"1m 12s"`.
pub(super) fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let mins = secs / 60;
    let rem = secs % 60;
    if mins == 0 {
        format!("{secs}s")
    } else if rem == 0 {
        format!("{mins}m")
    } else {
        format!("{mins}m {rem}s")
    }
}

pub(super) fn is_editing_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "write" | "edit" | "multiedit" | "apply_patch"
    )
}

pub(super) fn parse_exit_code(output: &str) -> Option<i32> {
    output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("exit_code="))
        .and_then(|code| code.trim().parse::<i32>().ok())
}

pub(super) fn parse_tool_payload(content: &str) -> Option<(bool, bool, String)> {
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

/// Extract the human-readable `output` field from a raw tool-result JSON payload.
pub(super) fn extract_tool_output_text(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(s) = v.get("output").and_then(|o| o.as_str()) {
            return s.to_string();
        }
    }
    raw.to_string()
}

/// Extract the most meaningful short argument from a tool call for inline display.
pub(super) fn extract_key_arg(tool: &str, arguments: &str) -> String {
    let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return String::new();
    };
    if tool.eq_ignore_ascii_case("apply_patch") {
        if let Some(patch_text) = args
            .get("patch_text")
            .or_else(|| args.get("patch"))
            .and_then(|v| v.as_str())
        {
            if let Some(path) = extract_patch_target(patch_text) {
                let display = tilde_path_in_line(&path);
                let short: String = display.chars().take(60).collect();
                return if short.len() < display.chars().count() {
                    format!("{short}…")
                } else {
                    short
                };
            }
            return "patch".to_string();
        }
    }
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
    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
        let short: String = cmd.chars().take(55).collect();
        return if short.len() < cmd.chars().count() {
            format!("{short}…")
        } else {
            short
        };
    }
    if let Some(q) = args
        .get("query")
        .or_else(|| args.get("pattern"))
        .and_then(|v| v.as_str())
    {
        return q.chars().take(55).collect();
    }
    if let Some(obj) = args.as_object() {
        for (key, val) in obj {
            if key == "contents" || key == "content" || key == "code" {
                continue;
            }
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    return s.chars().take(55).collect();
                }
            }
        }
    }
    let _ = tool;
    String::new()
}

pub(super) fn line_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}
