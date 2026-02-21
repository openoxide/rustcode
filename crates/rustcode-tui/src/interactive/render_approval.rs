use std::path::Path;

use super::syntax_highlight::{highlight_code_line, HighlightState};
use super::{
    Block, Borders, Color, Line, Modifier, Paragraph, Rect, Span, Style, ToolApprovalRequest, Wrap,
};

pub(super) fn approval_is_command_permission(permission: &str) -> bool {
    matches!(
        permission.to_ascii_lowercase().as_str(),
        "exec" | "bash" | "pty_exec"
    )
}

pub(super) fn approval_option_labels(req: &ToolApprovalRequest) -> Vec<String> {
    let mut options = vec![format!("Approve once [{}]", req.tool)];
    if approval_is_command_permission(&req.permission) {
        options.push("Allow all executionary commands".to_string());
    }
    options.push("Approve all tools (auto-pilot mode)".to_string());
    options.push("Deny".to_string());
    options
}

pub(super) fn approval_options_count(req: &ToolApprovalRequest) -> usize {
    approval_option_labels(req).len()
}

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
    let options = approval_option_labels(req);

    let mut option_lines: Vec<Line<'static>> = Vec::new();
    for (i, opt) in options.into_iter().enumerate() {
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
/// - `multiedit`: per-edit `-`/`+` snippet preview.
/// - `apply_patch`: unified/Codex patch preview with per-file diff lines.
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
    let tool = req.tool.to_ascii_lowercase();
    let w = width as usize;
    let mut out: Vec<Line<'static>> = Vec::new();

    // Extract file extension for syntax highlighting.
    let ext = args
        .get("path")
        .and_then(|v| v.as_str())
        .and_then(|p| p.rsplit('.').next())
        .unwrap_or("")
        .to_string();

    if tool == "apply_patch" {
        let patch_text = get_arg_str(args, &["patch_text", "patch"]);
        render_patch_text_preview(&mut out, &patch_text, &ext, w);
    } else if tool == "multiedit" {
        render_multiedit_preview(&mut out, args, &ext, w);
    } else if perm == "edit" {
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
                Span::styled(format!("{n:>3} "), Style::default().fg(Color::DarkGray)),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans(old_lines[i], &mut hl));
            out.push(padded_line(spans, Style::default(), w));
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
        let ctx_after_count = common_suffix
            .min(CONTEXT)
            .min(MAX_DISPLAY.saturating_sub(displayed));
        for i in 0..ctx_after_count {
            let old_idx = old_changed_end + i;
            let n = start_line + old_idx;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(Color::DarkGray)),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans(old_lines[old_idx], &mut hl));
            out.push(padded_line(spans, Style::default(), w));
            // (last context line, no need to track displayed further)
        }
        if common_suffix > ctx_after_count {
            out.push(Line::from(Span::styled(
                format!(
                    "     … {} unchanged lines below",
                    common_suffix - ctx_after_count
                ),
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
            let mut cmd_spans = vec![Span::styled("  $ ", Style::default().fg(Color::Yellow))];
            // Apply bash syntax highlighting to the command.
            let mut hl_bash = HighlightState::new("sh");
            cmd_spans.extend(code_spans(&short, &mut hl_bash));
            out.push(Line::from(cmd_spans));
        }
    }
    out
}

fn render_patch_text_preview(
    out: &mut Vec<Line<'static>>,
    patch_text: &str,
    default_ext: &str,
    width: usize,
) {
    if patch_text.trim().is_empty() {
        return;
    }
    const MAX_LINES: usize = 120;
    let mut current_ext = default_ext.to_string();
    let mut hl = HighlightState::new(&current_ext);
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut has_hunk_numbers = false;
    let mut rendered = 0usize;
    let mut truncated = false;

    for raw in patch_text.lines() {
        if rendered >= MAX_LINES {
            truncated = true;
            break;
        }
        let raw = raw.trim_end_matches('\r');
        let trimmed = raw.trim_start();

        if trimmed.starts_with("```") {
            continue;
        }

        if let Some(path) = patch_file_path_from_header(trimmed) {
            current_ext = file_ext(&path);
            hl = HighlightState::new(&current_ext);
            has_hunk_numbers = false;
            out.push(Line::from(vec![
                Span::styled("  file: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    path,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            rendered += 1;
            continue;
        }
        if trimmed.starts_with("--- ") {
            continue;
        }

        if let Some((old_start, new_start)) = parse_unified_hunk_starts(trimmed) {
            old_line = old_start;
            new_line = new_start;
            has_hunk_numbers = true;
            out.push(Line::from(Span::styled(
                format!("  {trimmed}"),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            )));
            rendered += 1;
            continue;
        }

        if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
            let mut spans = vec![Span::styled("  ", Style::default())];
            if has_hunk_numbers {
                spans.push(Span::styled(
                    format!("{new_line:>3} "),
                    Style::default().fg(Color::Rgb(80, 200, 80)),
                ));
            } else {
                spans.push(Span::styled("    ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(Span::styled(
                "+ ",
                Style::default().fg(Color::Rgb(80, 200, 80)),
            ));
            spans.extend(code_spans(trimmed.trim_start_matches('+'), &mut hl));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(3, 40, 0)),
                width,
            ));
            if has_hunk_numbers {
                new_line += 1;
            }
            rendered += 1;
        } else if trimmed.starts_with('-') && !trimmed.starts_with("---") {
            let mut spans = vec![Span::styled("  ", Style::default())];
            if has_hunk_numbers {
                spans.push(Span::styled(
                    format!("{old_line:>3} "),
                    Style::default().fg(Color::Rgb(200, 80, 80)),
                ));
            } else {
                spans.push(Span::styled("    ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(Span::styled(
                "- ",
                Style::default().fg(Color::Rgb(200, 80, 80)),
            ));
            spans.extend(code_spans(trimmed.trim_start_matches('-'), &mut hl));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(61, 1, 0)),
                width,
            ));
            if has_hunk_numbers {
                old_line += 1;
            }
            rendered += 1;
        } else if trimmed.starts_with("+++ ") || trimmed.starts_with("--- ") {
            out.push(Line::from(Span::styled(
                format!("  {trimmed}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));
            rendered += 1;
        } else if trimmed.starts_with("*** ") {
            continue;
        } else if let Some(rest) = trimmed.strip_prefix(' ') {
            let mut spans = vec![Span::styled("  ", Style::default())];
            if has_hunk_numbers {
                spans.push(Span::styled(
                    format!("{old_line:>3} "),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::styled("    ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(Span::styled("  ", Style::default()));
            spans.extend(code_spans(rest, &mut hl));
            out.push(padded_line(spans, Style::default(), width));
            if has_hunk_numbers {
                old_line += 1;
                new_line += 1;
            }
            rendered += 1;
        } else if trimmed.starts_with("\\ No newline") {
            continue;
        } else {
            out.push(Line::from(Span::styled(
                format!("  {trimmed}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
            rendered += 1;
        }
    }
    if truncated {
        out.push(Line::from(Span::styled(
            format!(
                "     … {} more patch lines",
                patch_text.lines().count().saturating_sub(rendered)
            ),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
    }
}

fn patch_file_path_from_header(line: &str) -> Option<String> {
    if let Some(path) = line.strip_prefix("+++ ") {
        let path = path.trim();
        if path != "/dev/null" && !path.is_empty() {
            return Some(path.trim_start_matches("b/").to_string());
        }
    }
    None
}

fn file_ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

fn parse_unified_hunk_starts(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, after_old) = rest.split_once(" +")?;
    let new_part = after_old.split_once(" @@")?.0;
    let old_start = old_part.split(',').next()?.parse::<usize>().ok()?;
    let new_start = new_part.split(',').next()?.parse::<usize>().ok()?;
    Some((old_start, new_start))
}

fn render_multiedit_preview(
    out: &mut Vec<Line<'static>>,
    args: &serde_json::Value,
    ext: &str,
    width: usize,
) {
    let raw_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    out.push(Line::from(vec![
        Span::styled("  file: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            tilde_path(raw_path),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let edits = args
        .get("edits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if edits.is_empty() {
        return;
    }

    const MAX_EDITS: usize = 6;
    const MAX_LINES_PER_SIDE: usize = 6;
    for (idx, edit) in edits.iter().take(MAX_EDITS).enumerate() {
        let old = edit
            .get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let new = edit
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let replace_all = edit
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(Line::from(Span::styled(
            format!(
                "  edit #{}{}",
                idx + 1,
                if replace_all { " (replace_all)" } else { "" }
            ),
            Style::default().fg(Color::DarkGray),
        )));

        let mut hl_del = HighlightState::new(ext);
        for raw in old.lines().take(MAX_LINES_PER_SIDE) {
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled("- ", Style::default().fg(Color::Rgb(200, 80, 80))),
            ];
            spans.extend(code_spans(raw, &mut hl_del));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(61, 1, 0)),
                width,
            ));
        }
        if old.lines().count() > MAX_LINES_PER_SIDE {
            out.push(Line::from(Span::styled(
                format!(
                    "     … {} more removed lines",
                    old.lines().count() - MAX_LINES_PER_SIDE
                ),
                Style::default()
                    .fg(Color::Rgb(200, 80, 80))
                    .add_modifier(Modifier::DIM),
            )));
        }

        let mut hl_add = HighlightState::new(ext);
        for raw in new.lines().take(MAX_LINES_PER_SIDE) {
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled("+ ", Style::default().fg(Color::Rgb(80, 200, 80))),
            ];
            spans.extend(code_spans(raw, &mut hl_add));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(3, 40, 0)),
                width,
            ));
        }
        if new.lines().count() > MAX_LINES_PER_SIDE {
            out.push(Line::from(Span::styled(
                format!(
                    "     … {} more added lines",
                    new.lines().count() - MAX_LINES_PER_SIDE
                ),
                Style::default()
                    .fg(Color::Rgb(80, 200, 80))
                    .add_modifier(Modifier::DIM),
            )));
        }
    }
    if edits.len() > MAX_EDITS {
        out.push(Line::from(Span::styled(
            format!("     … {} more edit blocks", edits.len() - MAX_EDITS),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_to_text(lines: &[Line<'_>]) -> String {
        let mut out = String::new();
        for line in lines {
            for span in &line.spans {
                out.push_str(span.content.as_ref());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn approval_preview_shows_apply_patch_diff_lines() {
        let req = ToolApprovalRequest {
            tool: "apply_patch".to_string(),
            permission: "write".to_string(),
            pattern: "src/main.rs".to_string(),
            arguments: serde_json::json!({
                "patch_text": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n"
            }),
            reason: String::new(),
        };

        let lines = render_approval_inline(&req, 120, Path::new("."));
        let text = lines_to_text(&lines);
        assert!(text.contains("@@ -1,2 +1,2 @@"), "text={text}");
        assert!(text.contains("- old"), "text={text}");
        assert!(text.contains("+ new"), "text={text}");
        assert!(text.contains("file: src/main.rs"), "text={text}");
    }

    #[test]
    fn approval_preview_shows_multiedit_blocks() {
        let req = ToolApprovalRequest {
            tool: "multiedit".to_string(),
            permission: "write".to_string(),
            pattern: "src/lib.rs".to_string(),
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "edits": [
                    { "old_string": "foo", "new_string": "bar", "replace_all": false }
                ]
            }),
            reason: String::new(),
        };

        let lines = render_approval_inline(&req, 120, Path::new("."));
        let text = lines_to_text(&lines);
        assert!(text.contains("edit #1"), "text={text}");
        assert!(text.contains("- foo"), "text={text}");
        assert!(text.contains("+ bar"), "text={text}");
    }
}
