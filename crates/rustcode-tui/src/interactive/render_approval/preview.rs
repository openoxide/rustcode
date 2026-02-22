use std::path::Path;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use rustcode_core::ToolApprovalRequest;

use super::super::syntax_highlight::{highlight_code_line, HighlightState};
use crate::interactive::theme;
mod patch;
use patch::render_patch_text_preview;

const EDIT_PREVIEW_MAX_DISPLAY: usize = 50;
const EDIT_PREVIEW_CONTEXT_LINES: usize = 2;
const MULTIEDIT_MAX_EDITS: usize = 6;
const MULTIEDIT_MAX_LINES_PER_SIDE: usize = 6;

/// Build diff-coloured preview lines for the approval inline block.
///
/// - `edit`:  proper line-level diff showing only changed lines with context.
/// - `write`: up to 30 green `+` lines with truncation hint.
/// - `multiedit`: per-edit `-`/`+` snippet preview.
/// - `apply_patch`: unified/Codex patch preview with per-file diff lines.
/// - `exec`:  a single dimmed `$ command` line.
///
/// `width` is the inner width of the transcript widget; each diff line is padded
/// with trailing spaces so the background colour fills the entire row.
pub(super) fn render_approval_preview(
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
        render_patch_text_preview(&mut out, &patch_text, &ext, w, workspace_root);
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

        // Show context before the change (last N lines of common prefix).
        let ctx_before_start = common_prefix.saturating_sub(EDIT_PREVIEW_CONTEXT_LINES);
        if ctx_before_start > 0 {
            out.push(Line::from(Span::styled(
                format!("     … {ctx_before_start} unchanged lines above"),
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }
        for (i, line) in old_lines
            .iter()
            .enumerate()
            .take(common_prefix)
            .skip(ctx_before_start)
        {
            if displayed >= EDIT_PREVIEW_MAX_DISPLAY {
                break;
            }
            let n = start_line + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(theme::MUTED)),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans_truncated(line, &mut hl, w));
            out.push(padded_line(spans, Style::default(), w));
            displayed += 1;
        }

        // Show deleted lines (old changed region).
        let del_count = old_changed_end - old_changed_start;
        let del_show = del_count.min(EDIT_PREVIEW_MAX_DISPLAY.saturating_sub(displayed));
        let mut hl_del = HighlightState::new(&ext);
        for i in 0..del_show {
            let n = start_line + old_changed_start + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(theme::DIFF_DEL_FG)),
                Span::styled("- ", Style::default().fg(theme::DIFF_DEL_FG)),
            ];
            spans.extend(code_spans_truncated(
                old_lines[old_changed_start + i],
                &mut hl_del,
                w,
            ));
            out.push(padded_line(
                spans,
                Style::default().bg(theme::DIFF_DEL_BG),
                w,
            ));
            displayed += 1;
        }
        if del_count > del_show {
            out.push(Line::from(Span::styled(
                format!("     … {} more deleted lines", del_count - del_show),
                Style::default()
                    .fg(theme::DIFF_DEL_FG)
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }

        // Show added lines (new changed region).
        let add_count = new_changed_end - new_changed_start;
        let add_show = add_count.min(EDIT_PREVIEW_MAX_DISPLAY.saturating_sub(displayed));
        let mut hl_add = HighlightState::new(&ext);
        for i in 0..add_show {
            let n = start_line + new_changed_start + i;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(theme::DIFF_ADD_FG)),
                Span::styled("+ ", Style::default().fg(theme::DIFF_ADD_FG)),
            ];
            spans.extend(code_spans_truncated(
                new_lines[new_changed_start + i],
                &mut hl_add,
                w,
            ));
            out.push(padded_line(
                spans,
                Style::default().bg(theme::DIFF_ADD_BG),
                w,
            ));
            displayed += 1;
        }
        if add_count > add_show {
            out.push(Line::from(Span::styled(
                format!("     … {} more added lines", add_count - add_show),
                Style::default()
                    .fg(theme::DIFF_ADD_FG)
                    .add_modifier(Modifier::DIM),
            )));
            displayed += 1;
        }

        // Show context after the change (first N lines of common suffix).
        let ctx_after_count = common_suffix
            .min(EDIT_PREVIEW_CONTEXT_LINES)
            .min(EDIT_PREVIEW_MAX_DISPLAY.saturating_sub(displayed));
        for i in 0..ctx_after_count {
            let old_idx = old_changed_end + i;
            let n = start_line + old_idx;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(theme::MUTED)),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans_truncated(old_lines[old_idx], &mut hl, w));
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
                    .fg(theme::MUTED)
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
            Span::styled("  new file: ", Style::default().fg(theme::MUTED)),
            Span::styled(
                display_path,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  (+{total_lines})"),
                Style::default().fg(theme::DIFF_ADD_FG),
            ),
        ]));

        let mut hl_write = HighlightState::new(&ext);
        for (i, raw) in content.lines().take(show_lines).enumerate() {
            let n = i + 1;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(theme::DIFF_ADD_FG)),
                Span::styled("+ ", Style::default().fg(theme::DIFF_ADD_FG)),
            ];
            spans.extend(code_spans_truncated(raw, &mut hl_write, w));
            out.push(padded_line(
                spans,
                Style::default().bg(theme::DIFF_ADD_BG),
                w,
            ));
        }
        if total_lines > show_lines {
            out.push(Line::from(Span::styled(
                format!("     … {} more lines", total_lines - show_lines),
                Style::default()
                    .fg(theme::DIFF_ADD_FG)
                    .add_modifier(Modifier::DIM),
            )));
        }
    } else {
        let cmd = get_arg_str(args, &["command", "cmd"]);
        if !cmd.is_empty() {
            let short: String = cmd.chars().take(80).collect();
            let mut cmd_spans = vec![Span::styled("  $ ", Style::default().fg(theme::WARNING))];
            // Apply bash syntax highlighting to the command.
            let mut hl_bash = HighlightState::new("sh");
            cmd_spans.extend(code_spans(&short, &mut hl_bash));
            out.push(Line::from(cmd_spans));
        }
    }
    out
}

pub(super) fn render_multiedit_preview(
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
        Span::styled("  file: ", Style::default().fg(theme::MUTED)),
        Span::styled(
            tilde_path(raw_path),
            Style::default()
                .fg(theme::TEXT)
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

    for (idx, edit) in edits.iter().take(MULTIEDIT_MAX_EDITS).enumerate() {
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
            Style::default().fg(theme::MUTED),
        )));

        let mut hl_del = HighlightState::new(ext);
        for raw in old.lines().take(MULTIEDIT_MAX_LINES_PER_SIDE) {
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled("- ", Style::default().fg(theme::DIFF_DEL_FG)),
            ];
            spans.extend(code_spans_truncated(raw, &mut hl_del, width));
            out.push(padded_line(
                spans,
                Style::default().bg(theme::DIFF_DEL_BG),
                width,
            ));
        }
        if old.lines().count() > MULTIEDIT_MAX_LINES_PER_SIDE {
            out.push(Line::from(Span::styled(
                format!(
                    "     … {} more removed lines",
                    old.lines().count() - MULTIEDIT_MAX_LINES_PER_SIDE
                ),
                Style::default()
                    .fg(theme::DIFF_DEL_FG)
                    .add_modifier(Modifier::DIM),
            )));
        }

        let mut hl_add = HighlightState::new(ext);
        for raw in new.lines().take(MULTIEDIT_MAX_LINES_PER_SIDE) {
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled("+ ", Style::default().fg(theme::DIFF_ADD_FG)),
            ];
            spans.extend(code_spans_truncated(raw, &mut hl_add, width));
            out.push(padded_line(
                spans,
                Style::default().bg(theme::DIFF_ADD_BG),
                width,
            ));
        }
        if new.lines().count() > MULTIEDIT_MAX_LINES_PER_SIDE {
            out.push(Line::from(Span::styled(
                format!(
                    "     … {} more added lines",
                    new.lines().count() - MULTIEDIT_MAX_LINES_PER_SIDE
                ),
                Style::default()
                    .fg(theme::DIFF_ADD_FG)
                    .add_modifier(Modifier::DIM),
            )));
        }
    }
    if edits.len() > MULTIEDIT_MAX_EDITS {
        out.push(Line::from(Span::styled(
            format!(
                "     … {} more edit blocks",
                edits.len() - MULTIEDIT_MAX_EDITS
            ),
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        )));
    }
}

/// Return syntax-highlighted spans for a code line, or a plain white fallback.
pub(super) fn code_spans(text: &str, hl: &mut Option<HighlightState>) -> Vec<Span<'static>> {
    if let Some(state) = hl.as_mut() {
        let spans = highlight_code_line(text, state);
        if !spans.is_empty() {
            return spans;
        }
    }
    vec![Span::styled(
        text.to_string(),
        Style::default().fg(theme::TEXT),
    )]
}

/// Like [`code_spans`] but truncates `text` to `max_width` chars first.
///
/// Avoids feeding extremely long lines (e.g. minified JS) into the
/// syntax highlighter which would allocate many spans and slow rendering.
pub(super) fn code_spans_truncated(
    text: &str,
    hl: &mut Option<HighlightState>,
    max_width: usize,
) -> Vec<Span<'static>> {
    if text.len() <= max_width {
        return code_spans(text, hl);
    }
    let truncated: String = text.chars().take(max_width).collect();
    let mut spans = code_spans(&truncated, hl);
    spans.push(Span::styled("…", Style::default().fg(theme::MUTED)));
    spans
}

/// Build a `Line` whose spans are padded with trailing spaces so the background
/// colour fills the full `width` of the rendering area.
pub(super) fn padded_line(
    mut spans: Vec<Span<'static>>,
    bg_style: Style,
    width: usize,
) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if width > used {
        spans.push(Span::styled(" ".repeat(width - used), bg_style));
    }
    Line::from(spans).style(bg_style)
}

pub(super) fn get_arg_str(args: &serde_json::Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = args.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    String::new()
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

/// Read the file at `args["path"]` (resolved against `workspace_root`) and
/// return the 1-based line number where `needle` first appears.
///
/// First attempts an exact substring match.  If that fails, falls back to
/// finding the first "distinctive" line of `needle` inside the file on a
/// trimmed basis, then back-calculates the start line.  Returns 1 if the
/// file cannot be read or the text cannot be located.
pub(super) fn find_text_start_line(
    workspace_root: &Path,
    args: &serde_json::Value,
    needle: &str,
) -> usize {
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

    // Fast path: exact substring match.
    if let Some(byte_offset) = contents.find(needle) {
        return contents[..byte_offset].lines().count() + 1;
    }

    // Fuzzy fallback: find the first distinctive line of needle in the file.
    // "Distinctive" means: trimmed length >= 6 and not composed entirely of
    // punctuation/brackets (avoids false matches on lines like `{`, `};`, etc.).
    let file_lines: Vec<&str> = contents.lines().collect();
    let needle_lines: Vec<&str> = needle.lines().collect();
    for (needle_idx, needle_line) in needle_lines.iter().enumerate() {
        let trimmed = needle_line.trim();
        if trimmed.len() < 6
            || trimmed
                .chars()
                .all(|c| "{}();,[]<>/*+-=|&^%#@!~`".contains(c) || c.is_whitespace())
        {
            continue;
        }
        // Search for this trimmed line in the file.
        for (file_idx, file_line) in file_lines.iter().enumerate() {
            if file_line.trim() == trimmed {
                // Back-calculate where the needle starts in the file.
                let start = file_idx.saturating_sub(needle_idx) + 1;
                return start;
            }
        }
        // Only attempt the first distinctive line to avoid wrong matches.
        break;
    }

    1
}
