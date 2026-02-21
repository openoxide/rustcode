use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::{code_spans, padded_line};

const PATCH_MAX_LINES: usize = 120;

/// Render a patch preview using the same `  NNN +/-/  code` style as the
/// `write` and `edit` approval previews.  Accepts unified diff and Codex
/// `*** Update File:` format.
pub(super) fn render_patch_text_preview(
    out: &mut Vec<Line<'static>>,
    patch_text: &str,
    default_ext: &str,
    width: usize,
) {
    use super::super::super::syntax_highlight::HighlightState;

    if patch_text.trim().is_empty() {
        return;
    }
    let mut current_ext = default_ext.to_string();
    let mut hl = HighlightState::new(&current_ext);
    let mut add_no = 0usize;
    let mut del_no = 0usize;
    let mut rendered = 0usize;
    let mut truncated = false;

    for raw in patch_text.lines() {
        if rendered >= PATCH_MAX_LINES {
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

        if let Some(path) = trimmed
            .strip_prefix("*** Update File: ")
            .or_else(|| trimmed.strip_prefix("*** Add File: "))
        {
            let path = path.trim();
            current_ext = file_ext(path);
            hl = HighlightState::new(&current_ext);
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

        if trimmed == "@@" {
            continue;
        }

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
    use std::path::Path;
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
