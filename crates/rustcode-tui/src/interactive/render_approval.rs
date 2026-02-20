use std::path::Path;

use super::syntax_highlight::{highlight_code_line, HighlightState};
use super::{
    Block, Borders, Color, Line, Modifier, Paragraph, Rect, Span, Style, ToolApprovalRequest, Wrap,
};

/// Build approval info lines to append to the transcript when a tool approval is pending.
///
/// Shows the action title, reason (if any), and a short content preview (up to 8 lines).
/// `width` is the inner width of the transcript area so diff backgrounds fill the full row.
pub(super) fn render_approval_inline(
    req: &ToolApprovalRequest,
    width: u16,
    workspace_root: &Path,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::raw("")));
    // Show diff preview first, then the approval title/reason below it.
    lines.extend(render_approval_preview(req, width, workspace_root));
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(vec![
        Span::styled(
            "  Approval  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            approval_action_title(req),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    if !req.reason.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  Reason: {}", req.reason),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    lines
}

/// Render the approval selector widget in place of the composer.
///
/// Shows ←/→ navigable options inside a yellow-bordered "Approval" block.
/// The `selected` index highlights the active option with a reversed-video indicator.
pub(super) fn render_approval_selector(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    req: &ToolApprovalRequest,
    selected: usize,
) {
    let perm = req.permission.to_lowercase();
    let is_edit = perm == "write" || perm == "edit";
    let is_cmd = perm == "exec";
    let options: &[&str] = if is_edit {
        &["Allow once", "Allow all edits", "Deny"]
    } else if is_cmd {
        &["Allow once", "Allow all cmds", "Deny"]
    } else {
        &["Approve", "Deny"]
    };

    let mut option_lines: Vec<Line<'static>> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        if i == selected {
            option_lines.push(Line::from(Span::styled(
                format!("  ❯ {opt}"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            )));
        } else {
            option_lines.push(Line::from(Span::styled(
                format!("    {opt}"),
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
    }
    option_lines.push(Line::from(Span::styled(
        "  ↑/↓ navigate   Enter: confirm   Esc: deny",
        Style::default().add_modifier(Modifier::DIM),
    )));

    frame.render_widget(
        Paragraph::new(option_lines)
        .block(
            Block::default()
                .title(Span::styled(
                    "Approval",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn approval_action_title(req: &ToolApprovalRequest) -> String {
    let perm = req.permission.to_lowercase();
    let raw_path = req
        .arguments
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.pattern);
    // Show full path with ~ replacing $HOME.
    let display_path = tilde_path(raw_path);
    if perm == "write" || perm == "edit" {
        format!("Write / edit:  {}", display_path)
    } else if perm == "exec" {
        "Run command".to_string()
    } else {
        req.tool.clone()
    }
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

/// Build diff-coloured preview lines for the approval inline block.
///
/// - `edit`:  proper line-level diff showing only changed lines with context.
/// - `write`: up to 10 green `+` lines with truncation hint.
/// - `exec`:  a single dimmed `$ command` line.
///
/// `width` is the inner width of the transcript widget; each diff line is padded
/// with trailing spaces so the background colour fills the entire row.
fn render_approval_preview(
    req: &ToolApprovalRequest,
    width: u16,
    workspace_root: &Path,
) -> Vec<Line<'static>> {
    let args = &req.arguments;
    let perm = req.permission.to_lowercase();
    let w = width as usize;
    let mut out: Vec<Line<'static>> = Vec::new();

    // Extract file extension for syntax highlighting.
    let ext = args
        .get("path")
        .and_then(|v| v.as_str())
        .and_then(|p| p.rsplit('.').next())
        .unwrap_or("")
        .to_string();

    if perm == "edit" {
        let old = get_arg_str(args, &["from", "old_str", "old_string", "old"]);
        let new = get_arg_str(args, &["to", "new_str", "new_string", "new"]);
        let start_line = find_text_start_line(workspace_root, args, &old);

        // Compute a proper line-level diff between old and new.
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        // Find common prefix length.
        let common_prefix = old_lines
            .iter()
            .zip(new_lines.iter())
            .take_while(|(a, b)| a == b)
            .count();

        // Find common suffix length (that doesn't overlap with prefix).
        let max_suffix = old_lines.len().min(new_lines.len()) - common_prefix;
        let common_suffix = old_lines
            .iter()
            .rev()
            .zip(new_lines.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();

        // The changed region in old and new.
        let old_changed_start = common_prefix;
        let old_changed_end = old_lines.len() - common_suffix;
        let new_changed_start = common_prefix;
        let new_changed_end = new_lines.len() - common_suffix;

        let mut hl = HighlightState::new(&ext);
        let mut displayed = 0usize;
        const MAX_DISPLAY: usize = 50;
        const CONTEXT: usize = 2;

        // Show context before the change (last N lines of common prefix).
        let ctx_before_start = if common_prefix > CONTEXT {
            common_prefix - CONTEXT
        } else {
            0
        };
        if ctx_before_start > 0 {
            out.push(Line::from(Span::styled(
                format!("     … {} unchanged lines above", ctx_before_start),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }
        for i in ctx_before_start..common_prefix {
            if displayed >= MAX_DISPLAY {
                break;
            }
            let n = start_line + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans(old_lines[i], &mut hl));
            out.push(padded_line(
                spans,
                Style::default(),
                w,
            ));
            displayed += 1;
        }

        // Show deleted lines (old changed region).
        let del_count = old_changed_end - old_changed_start;
        let del_show = del_count.min(MAX_DISPLAY.saturating_sub(displayed));
        let mut hl_del = HighlightState::new(&ext);
        for i in 0..del_show {
            let n = start_line + old_changed_start + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::Rgb(200, 80, 80)),
                ),
                Span::styled("- ", Style::default().fg(Color::Rgb(200, 80, 80))),
            ];
            spans.extend(code_spans(old_lines[old_changed_start + i], &mut hl_del));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(61, 1, 0)),
                w,
            ));
            displayed += 1;
        }
        if del_count > del_show {
            out.push(Line::from(Span::styled(
                format!("     … {} more deleted lines", del_count - del_show),
                Style::default()
                    .fg(Color::Rgb(200, 80, 80))
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }

        // Show added lines (new changed region).
        let add_count = new_changed_end - new_changed_start;
        let add_show = add_count.min(MAX_DISPLAY.saturating_sub(displayed));
        let mut hl_add = HighlightState::new(&ext);
        for i in 0..add_show {
            let n = start_line + new_changed_start + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::Rgb(80, 200, 80)),
                ),
                Span::styled("+ ", Style::default().fg(Color::Rgb(80, 200, 80))),
            ];
            spans.extend(code_spans(new_lines[new_changed_start + i], &mut hl_add));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(3, 40, 0)),
                w,
            ));
            displayed += 1;
        }
        if add_count > add_show {
            out.push(Line::from(Span::styled(
                format!("     … {} more added lines", add_count - add_show),
                Style::default()
                    .fg(Color::Rgb(80, 200, 80))
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }

        // Show context after the change (first N lines of common suffix).
        let ctx_after_count = common_suffix.min(CONTEXT).min(MAX_DISPLAY.saturating_sub(displayed));
        for i in 0..ctx_after_count {
            let old_idx = old_changed_end + i;
            let n = start_line + old_idx;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans(old_lines[old_idx], &mut hl));
            out.push(padded_line(
                spans,
                Style::default(),
                w,
            ));
            // (last context line, no need to track displayed further)
        }
        if common_suffix > ctx_after_count {
            out.push(Line::from(Span::styled(
                format!("     … {} unchanged lines below", common_suffix - ctx_after_count),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
    } else if perm == "write" {
        let content = get_arg_str(args, &["contents", "content", "code"]);
        let total_lines = content.lines().count();
        let show_lines = total_lines.min(30);

        // File header for new file creation.
        let raw_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("new file");
        let display_path = tilde_path(raw_path);
        out.push(Line::from(vec![
            Span::styled("  new file: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                display_path,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  (+{})", total_lines),
                Style::default().fg(Color::Rgb(80, 200, 80)),
            ),
        ]));

        let mut hl_write = HighlightState::new(&ext);
        for (i, raw) in content.lines().take(show_lines).enumerate() {
            let n = i + 1;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::Rgb(80, 200, 80)),
                ),
                Span::styled("+ ", Style::default().fg(Color::Rgb(80, 200, 80))),
            ];
            spans.extend(code_spans(raw, &mut hl_write));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(3, 40, 0)),
                w,
            ));
        }
        if total_lines > show_lines {
            out.push(Line::from(Span::styled(
                format!("     … {} more lines", total_lines - show_lines),
                Style::default()
                    .fg(Color::Rgb(80, 200, 80))
                    .add_modifier(Modifier::DIM),
            )));
        }
    } else {
        let cmd = get_arg_str(args, &["command", "cmd"]);
        if !cmd.is_empty() {
            let short: String = cmd.chars().take(80).collect();
            let mut cmd_spans = vec![
                Span::styled("  $ ", Style::default().fg(Color::Yellow)),
            ];
            // Apply bash syntax highlighting to the command.
            let mut hl_bash = HighlightState::new("sh");
            cmd_spans.extend(code_spans(&short, &mut hl_bash));
            out.push(Line::from(cmd_spans));
        }
    }
    out
}

/// Return syntax-highlighted spans for a code line, or a plain white fallback.
fn code_spans(text: &str, hl: &mut Option<HighlightState>) -> Vec<Span<'static>> {
    if let Some(state) = hl.as_mut() {
        let spans = highlight_code_line(text, state);
        if !spans.is_empty() {
            return spans;
        }
    }
    vec![Span::styled(
        text.to_string(),
        Style::default().fg(Color::White),
    )]
}

/// Build a `Line` whose spans are padded with trailing spaces so the background
/// colour fills the full `width` of the rendering area.
fn padded_line(mut spans: Vec<Span<'static>>, bg_style: Style, width: usize) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if width > used {
        spans.push(Span::styled(" ".repeat(width - used), bg_style));
    }
    Line::from(spans).style(bg_style)
}

fn get_arg_str(args: &serde_json::Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = args.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    String::new()
}

/// Read the file at `args["path"]` (resolved against `workspace_root`) and
/// return the 1-based line number where `needle` first appears.
/// Falls back to 1 if the file can't be read or the text isn't found.
fn find_text_start_line(workspace_root: &Path, args: &serde_json::Value, needle: &str) -> usize {
    if needle.is_empty() {
        return 1;
    }
    let Some(rel_path) = args.get("path").and_then(|v| v.as_str()) else {
        return 1;
    };
    let abs_path = workspace_root.join(rel_path);
    let Ok(contents) = std::fs::read_to_string(&abs_path) else {
        return 1;
    };
    // Find the byte offset of the needle in the file.
    if let Some(byte_offset) = contents.find(needle) {
        // Count newlines before that offset to get the 1-based line number.
        contents[..byte_offset].lines().count() + 1
    } else {
        1
    }
}
