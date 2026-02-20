use std::sync::LazyLock;

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

/// Global syntax and theme sets — loaded once on first use.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Persistent state for multi-line highlighting so strings/comments that span
/// multiple lines are coloured correctly.
pub(super) struct HighlightState {
    highlighter: HighlightLines<'static>,
}

impl HighlightState {
    /// Create a new highlighting state for the given file extension.
    ///
    /// Returns `None` if the extension is not recognised by syntect.
    pub(super) fn new(extension: &str) -> Option<Self> {
        let syntax = find_syntax(extension)?;
        let theme = THEME_SET.themes.get("base16-eighties.dark")?;
        let hl = HighlightLines::new(syntax, theme);
        Some(Self { highlighter: hl })
    }
}

/// Look up a syntax definition by file extension.
fn find_syntax(extension: &str) -> Option<&'static SyntaxReference> {
    let ext = extension.trim_start_matches('.');
    SYNTAX_SET.find_syntax_by_extension(ext)
}

/// Highlight a single code line and return coloured spans.
///
/// The returned spans have transparent backgrounds so the caller's diff
/// background colour shows through. If highlighting fails, a single
/// white-foreground span is returned as a fallback.
pub(super) fn highlight_code_line(
    text: &str,
    state: &mut HighlightState,
) -> Vec<Span<'static>> {
    // The syntax set was loaded with `load_defaults_newlines`, so each line
    // passed to `highlight_line` MUST end with `\n` — otherwise single-line
    // scopes (like Python `#` comments) never close and bleed into later lines.
    let with_nl = format!("{text}\n");
    let ranges = match state.highlighter.highlight_line(&with_nl, &SYNTAX_SET) {
        Ok(r) => r,
        Err(_) => {
            return vec![Span::styled(
                text.to_string(),
                Style::default().fg(Color::White),
            )]
        }
    };

    if ranges.is_empty() {
        return vec![Span::styled(
            text.to_string(),
            Style::default().fg(Color::White),
        )];
    }

    ranges
        .into_iter()
        .filter(|(_, s)| !s.is_empty())
        .map(|(style, slice)| {
            Span::styled(
                slice.to_string(),
                Style::default().fg(syntect_to_ratatui_color(style.foreground)),
            )
        })
        .collect()
}

/// Convert a syntect RGBA colour to a ratatui `Color`, boosting dim colours
/// so that comments (grey) remain readable against diff backgrounds.
fn syntect_to_ratatui_color(c: syntect::highlighting::Color) -> Color {
    // Boost very dim foreground colors so they stay readable.
    let max_ch = c.r.max(c.g).max(c.b);
    if max_ch < 100 {
        // Too dim — lift all channels proportionally.
        let scale = 140.0 / (max_ch as f32).max(1.0);
        let r = ((c.r as f32) * scale).min(255.0) as u8;
        let g = ((c.g as f32) * scale).min(255.0) as u8;
        let b = ((c.b as f32) * scale).min(255.0) as u8;
        Color::Rgb(r, g, b)
    } else {
        Color::Rgb(c.r, c.g, c.b)
    }
}
