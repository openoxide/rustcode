/// Lightweight markdown → ratatui `Line` renderer.
///
/// Supports:
/// - Headings: `# `, `## `, `### `
/// - Code fences: ` ``` ` (whole block)
/// - Bullet lists: `- `, `* `, `+ `, `N. `
/// - Horizontal rules: `---` / `===` (3+ chars)
/// - Inline: `**bold**`, `*italic*`, `` `code` ``
/// - Plain text passthrough
use super::syntax_highlight::{highlight_code_line, HighlightState};
use super::theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Parse a markdown string and return styled ratatui `Line`s.
pub(super) fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_hl: Option<HighlightState> = None;

    for raw in text.lines() {
        let stripped = raw.trim_end();

        // ── Code fence toggle ──────────────────────────────────────
        if let Some(fence_rest) = stripped.strip_prefix("```") {
            if in_code_block {
                in_code_block = false;
                code_lang.clear();
                code_hl = None;
                lines.push(Line::from(vec![Span::styled(
                    "─".repeat(40),
                    Style::default().fg(theme::BORDER),
                )]));
            } else {
                in_code_block = true;
                code_lang = fence_rest.trim().to_string();
                let label = if code_lang.is_empty() {
                    "code".to_string()
                } else {
                    code_lang.clone()
                };
                code_hl = HighlightState::new(&code_lang);
                lines.push(Line::from(vec![
                    Span::styled("─── ", Style::default().fg(theme::BORDER)),
                    Span::styled(
                        label,
                        Style::default()
                            .fg(theme::TEXT)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ───", Style::default().fg(theme::BORDER)),
                ]));
            }
            continue;
        }

        if in_code_block {
            let mut code_line_spans = vec![Span::raw("  ".to_string())];
            if let Some(ref mut hl) = code_hl {
                let hl_spans = highlight_code_line(stripped, hl);
                if hl_spans.is_empty() {
                    code_line_spans.push(Span::styled(
                        stripped.to_string(),
                        Style::default().fg(theme::TEXT_DIM),
                    ));
                } else {
                    code_line_spans.extend(hl_spans);
                }
            } else {
                code_line_spans.push(Span::styled(
                    stripped.to_string(),
                    Style::default().fg(theme::TEXT_DIM),
                ));
            }
            lines.push(Line::from(code_line_spans));
            continue;
        }

        // ── Horizontal rule ────────────────────────────────────────
        if stripped.len() >= 3
            && (stripped.chars().all(|c| c == '-') || stripped.chars().all(|c| c == '='))
        {
            lines.push(Line::from(vec![Span::styled(
                "─".repeat(stripped.len().min(80)),
                Style::default().fg(theme::BORDER),
            )]));
            continue;
        }

        // ── Headings ───────────────────────────────────────────────
        if let Some(rest) = stripped.strip_prefix("### ") {
            let spans = restyle_spans(
                parse_inline(rest),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::DIM),
            );
            lines.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = stripped.strip_prefix("## ") {
            let spans = restyle_spans(
                parse_inline(rest),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            );
            lines.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = stripped.strip_prefix("# ") {
            let spans = restyle_spans(
                parse_inline_bold(rest),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            );
            lines.push(Line::from(spans));
            continue;
        }

        // ── Bullet lists ───────────────────────────────────────────
        let (indent, rest) = leading_spaces(stripped);
        let indent_str = " ".repeat(indent);
        if let Some(item) = rest
            .strip_prefix("- ")
            .or_else(|| rest.strip_prefix("* ").or_else(|| rest.strip_prefix("+ ")))
        {
            let mut spans = vec![
                Span::raw(indent_str.clone()),
                Span::styled("• ", Style::default().fg(theme::ACCENT)),
            ];
            spans.extend(parse_inline(item));
            lines.push(Line::from(spans));
            continue;
        }

        // Numbered list: `1. `, `2. ` …
        if let Some((num, item)) = try_numbered_list(rest) {
            let mut spans = vec![
                Span::raw(indent_str),
                Span::styled(
                    format!("{num}. "),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            spans.extend(parse_inline(item));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Blank line ─────────────────────────────────────────────
        if stripped.is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        // ── Plain paragraph ────────────────────────────────────────
        let spans = parse_inline(stripped);
        lines.push(Line::from(spans));
    }

    // Close unterminated code block
    if in_code_block {
        lines.push(Line::from(vec![Span::styled(
            "─".repeat(40),
            Style::default().fg(theme::BORDER),
        )]));
    }

    lines
}

/// Parse inline markdown spans (bold, italic, inline code).
fn parse_inline(text: &str) -> Vec<Span<'static>> {
    parse_inline_with_base(text, Style::default())
}

fn parse_inline_bold(text: &str) -> Vec<Span<'static>> {
    parse_inline_with_base(text, Style::default().add_modifier(Modifier::BOLD))
}

fn parse_inline_with_base(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = text.chars().peekable();
    let mut buf = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            // Inline code: `code`
            '`' => {
                flush(&mut spans, &mut buf, base);
                let mut code = String::new();
                for c in chars.by_ref() {
                    if c == '`' {
                        break;
                    }
                    code.push(c);
                }
                if !code.is_empty() {
                    spans.push(Span::styled(
                        code,
                        Style::default()
                            .fg(theme::TEXT_DIM)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }
            // Bold: **text** or __text__
            '*' | '_' if chars.peek() == Some(&ch) => {
                chars.next(); // consume second * or _
                flush(&mut spans, &mut buf, base);
                let delim = if ch == '*' { "**" } else { "__" };
                let end = delim;
                let mut bold = String::new();
                let mut prev = ' ';
                loop {
                    match chars.next() {
                        Some(c) if c == ch && prev == ch => {
                            bold.pop(); // remove the doubled char that was added
                            break;
                        }
                        Some(c) => {
                            bold.push(c);
                            prev = c;
                        }
                        None => {
                            // Unterminated — treat as literal
                            buf.push_str(end);
                            buf.push_str(&bold);
                            bold.clear();
                            break;
                        }
                    }
                }
                if !bold.is_empty() {
                    spans.push(Span::styled(bold, base.add_modifier(Modifier::BOLD)));
                }
            }
            // Italic: *text* or _text_
            '*' | '_' => {
                flush(&mut spans, &mut buf, base);
                let end = ch;
                let mut italic = String::new();
                loop {
                    match chars.next() {
                        Some(c) if c == end => break,
                        Some(c) => italic.push(c),
                        None => {
                            buf.push(end);
                            buf.push_str(&italic);
                            italic.clear();
                            break;
                        }
                    }
                }
                if !italic.is_empty() {
                    spans.push(Span::styled(italic, base.add_modifier(Modifier::ITALIC)));
                }
            }
            other => buf.push(other),
        }
    }

    flush(&mut spans, &mut buf, base);
    spans
}

fn flush(spans: &mut Vec<Span<'static>>, buf: &mut String, style: Style) {
    if !buf.is_empty() {
        spans.push(Span::styled(buf.clone(), style));
        buf.clear();
    }
}

fn restyle_spans(spans: Vec<Span<'static>>, style: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style.patch(style)))
        .collect()
}

/// Returns `(indent_spaces, rest)` stripping leading spaces.
fn leading_spaces(s: &str) -> (usize, &str) {
    let trimmed = s.trim_start_matches(' ');
    let indent = s.len() - trimmed.len();
    (indent, trimmed)
}

/// Try to parse a numbered list item like `1. text`.
fn try_numbered_list(s: &str) -> Option<(u32, &str)> {
    let dot = s.find(". ")?;
    let num_part = &s[..dot];
    if num_part.is_empty() || num_part.len() > 4 {
        return None;
    }
    let num: u32 = num_part.parse().ok()?;
    Some((num, &s[dot + 2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_plain_text() {
        let lines = render_markdown("hello world");
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn renders_heading() {
        let lines = render_markdown("# Hello");
        assert_eq!(lines.len(), 1);
        let first = &lines[0];
        assert!(first.spans.iter().any(|span| span.content == "Hello"));
        assert!(!first.spans.iter().any(|span| span.content == "# "));
    }

    #[test]
    fn renders_code_block() {
        let lines = render_markdown("```rust\nlet x = 1;\n```");
        // fence-open + code line + fence-close = 3 lines
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn renders_bullet_list() {
        let lines = render_markdown("- item one\n- item two");
        assert_eq!(lines.len(), 2);
        // Each bullet line has the bullet span
        assert!(lines[0].spans[1].content == "• ");
    }

    #[test]
    fn renders_blank_line() {
        let lines = render_markdown("a\n\nb");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn parse_inline_bold() {
        let spans = parse_inline("hello **world** end");
        assert!(spans.len() >= 3);
    }

    #[test]
    fn parse_inline_code() {
        let spans = parse_inline("use `cargo test`");
        // has a code span styled with TEXT_DIM
        assert!(spans.iter().any(|s| s.style.fg == Some(theme::TEXT_DIM)));
    }

    #[test]
    fn numbered_list() {
        let lines = render_markdown("1. first\n2. second");
        assert_eq!(lines.len(), 2);
    }
}
