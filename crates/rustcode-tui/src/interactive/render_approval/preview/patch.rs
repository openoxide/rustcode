use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{code_spans, padded_line};

const PATCH_MAX_LINES: usize = 120;

/// Render a patch preview using the same `  NNN +/-/  code` style as the
/// `write` and `edit` approval previews.  Accepts unified diff and Codex
/// `*** Update File:` format.
///
/// For Codex-format patches (bare `@@` without line numbers), the function
/// reads the actual file from `workspace_root` to determine the correct
/// starting line offset so diff numbers match the real file.
pub(super) fn render_patch_text_preview(
    out: &mut Vec<Line<'static>>,
    patch_text: &str,
    default_ext: &str,
    width: usize,
    workspace_root: &Path,
) {
    use super::super::super::syntax_highlight::HighlightState;

    if patch_text.trim().is_empty() {
        return;
    }

    let all_lines: Vec<&str> = patch_text.lines().collect();
    let mut current_ext = default_ext.to_string();
    let mut hl = HighlightState::new(&current_ext);
    let mut add_no = 0usize;
    let mut del_no = 0usize;
    let mut rendered = 0usize;
    let mut truncated = false;
    let mut current_file_path = String::new();

    for (line_idx, raw) in all_lines.iter().enumerate() {
        if rendered >= PATCH_MAX_LINES {
            truncated = true;
            break;
        }
        let raw = raw.trim_end_matches('\r');
        let trimmed = raw.trim_start();

        if trimmed.starts_with("```") {
            continue;
        }

        // Unified diff file header: +++ b/path
        if let Some(path) = patch_file_path_from_header(trimmed) {
            current_ext = file_ext(&path);
            hl = HighlightState::new(&current_ext);
            current_file_path = path.clone();
            add_no = 0;
            del_no = 0;
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

        // Codex file header: *** Update File: / *** Add File:
        if let Some(path) = trimmed
            .strip_prefix("*** Update File: ")
            .or_else(|| trimmed.strip_prefix("*** Add File: "))
        {
            let path = path.trim();
            current_ext = file_ext(path);
            hl = HighlightState::new(&current_ext);
            current_file_path = path.to_string();
            add_no = 0;
            del_no = 0;
            out.push(Line::from(vec![
                Span::styled("  file: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    path.to_string(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            rendered += 1;
            continue;
        }

        if trimmed.starts_with("*** ") || trimmed.starts_with("--- ") {
            continue;
        }

        // Codex bare @@ hunk marker — resolve line offset from the actual file.
        if trimmed == "@@" {
            if !current_file_path.is_empty() {
                if let Some(offset) = find_codex_hunk_offset(
                    &all_lines,
                    line_idx + 1,
                    workspace_root,
                    &current_file_path,
                ) {
                    add_no = offset;
                    del_no = offset;
                    out.push(Line::from(Span::styled(
                        format!("     … line {}", offset + 1),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    )));
                    rendered += 1;
                }
            }
            continue;
        }

        // Unified hunk header: @@ -X,Y +A,B @@
        if let Some((old_start, new_start)) = parse_unified_hunk_starts(trimmed) {
            del_no = old_start.saturating_sub(1);
            add_no = new_start.saturating_sub(1);
            out.push(Line::from(Span::styled(
                format!("     … line {new_start}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
            rendered += 1;
            continue;
        }

        if trimmed.starts_with("\\ No newline") {
            continue;
        }

        if raw.starts_with('+') {
            add_no += 1;
            let code_text = raw.get(1..).unwrap_or("");
            let n = add_no;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::Rgb(80, 200, 80)),
                ),
                Span::styled("+ ", Style::default().fg(Color::Rgb(80, 200, 80))),
            ];
            spans.extend(code_spans(code_text, &mut hl));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(3, 40, 0)),
                width,
            ));
            rendered += 1;
            continue;
        }

        if raw.starts_with('-') {
            del_no += 1;
            let code_text = raw.get(1..).unwrap_or("");
            let n = del_no;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{n:>3} "),
                    Style::default().fg(Color::Rgb(200, 80, 80)),
                ),
                Span::styled("- ", Style::default().fg(Color::Rgb(200, 80, 80))),
            ];
            spans.extend(code_spans(code_text, &mut hl));
            out.push(padded_line(
                spans,
                Style::default().bg(Color::Rgb(61, 1, 0)),
                width,
            ));
            rendered += 1;
            continue;
        }

        if let Some(code_text) = raw.strip_prefix(' ') {
            add_no += 1;
            del_no += 1;
            let n = add_no;
            let mut spans = vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{n:>3} "), Style::default().fg(Color::DarkGray)),
                Span::styled("  ", Style::default()),
            ];
            spans.extend(code_spans(code_text, &mut hl));
            out.push(padded_line(spans, Style::default(), width));
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

/// For a Codex `@@` hunk (no line numbers), look ahead to find the first
/// few deletion / context lines, then search for them in the actual file to
/// determine the 0-based line offset for the counters.
fn find_codex_hunk_offset(
    lines: &[&str],
    start_idx: usize,
    workspace_root: &Path,
    file_path: &str,
) -> Option<usize> {
    let abs_path = workspace_root.join(file_path);
    let contents = std::fs::read_to_string(&abs_path).ok()?;
    let file_lines: Vec<&str> = contents.lines().collect();

    // Collect context and deletion lines (the "before" content).
    let mut search_lines: Vec<&str> = Vec::new();
    for raw in &lines[start_idx..] {
        let trimmed = raw.trim_start();
        // Stop at next hunk / file header / end-of-patch marker.
        if trimmed.starts_with("*** ")
            || trimmed.starts_with("@@ ")
            || trimmed == "@@"
            || trimmed.starts_with("+++ ")
            || trimmed.starts_with("--- ")
        {
            break;
        }
        if raw.starts_with('-') {
            search_lines.push(raw.get(1..).unwrap_or(""));
        } else if raw.starts_with(' ') {
            search_lines.push(raw.get(1..).unwrap_or(""));
        }
        // Skip '+' lines — they are new content, not in the file yet.
    }

    if search_lines.is_empty() {
        return None;
    }

    // Try to find a distinctive needle line in the file.
    for (needle_idx, needle_line) in search_lines.iter().enumerate() {
        let trimmed = needle_line.trim();
        if trimmed.len() < 4
            || trimmed
                .chars()
                .all(|c| "{}();,[]<>/*+-=|&^%#@!~`'\"".contains(c) || c.is_whitespace())
        {
            continue;
        }
        for (file_idx, file_line) in file_lines.iter().enumerate() {
            if file_line.trim() == trimmed {
                return Some(file_idx.saturating_sub(needle_idx));
            }
        }
        // Only try the first distinctive line to avoid false matches.
        break;
    }

    None
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
