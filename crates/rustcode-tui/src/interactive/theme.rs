//! Tokyo Night color theme for the `RustCode` TUI.
//!
//! All color constants live here so the palette can be adjusted in one place.
//! Every constant uses `Color::Rgb(r, g, b)` for visual consistency.

use ratatui::style::Color;

// ── Core palette ─────────────────────────────────────────────────────

/// Primary accent — soft blue. Headings, active borders, key hints,
/// assistant markers, tool call indicators, focused UI elements.
pub(crate) const ACCENT: Color = Color::Rgb(122, 162, 247);

/// Secondary accent — soft purple. Brand label, tool tags,
/// skill/details modal borders.
pub(crate) const SECONDARY: Color = Color::Rgb(187, 154, 247);

/// Success / added — muted green. Success indicators, diff add text,
/// git insertions, completed bullets.
pub(crate) const SUCCESS: Color = Color::Rgb(158, 206, 106);

/// Error / deleted — soft red. Error indicators, diff delete text,
/// git deletions, failed bullets.
pub(crate) const ERROR: Color = Color::Rgb(247, 118, 142);

/// Warning — soft orange. Approval borders, model select accent,
/// mode labels.
pub(crate) const WARNING: Color = Color::Rgb(224, 175, 104);

/// Info — teal. Provider labels, info toasts, reasoning/thinking.
pub(crate) const INFO: Color = Color::Rgb(125, 207, 255);

// ── Text hierarchy ───────────────────────────────────────────────────

/// Primary text — silver. File names, code fallback, main content.
pub(crate) const TEXT: Color = Color::Rgb(192, 202, 245);

/// Dimmed / secondary text — warm gray. Context code lines,
/// inline code in markdown, code block fallback.
pub(crate) const TEXT_DIM: Color = Color::Rgb(169, 177, 214);

/// Muted text — slate gray. Gutter numbers, tool output summaries,
/// hint descriptions, elapsed time, pending spinners.
pub(crate) const MUTED: Color = Color::Rgb(86, 95, 137);

// ── UI chrome ────────────────────────────────────────────────────────

/// Active / focused border.
pub(crate) const BORDER_FOCUS: Color = Color::Rgb(122, 162, 247);

/// Inactive / unfocused border.
pub(crate) const BORDER: Color = Color::Rgb(59, 66, 97);

/// Surface — very dark blue. Hunk separators, deep background accents.
pub(crate) const SURFACE: Color = Color::Rgb(41, 46, 66);

// ── Diff colors ──────────────────────────────────────────────────────

/// Diff added line foreground (green text).
pub(crate) const DIFF_ADD_FG: Color = Color::Rgb(158, 206, 106);

/// Diff added line background (dark green).
pub(crate) const DIFF_ADD_BG: Color = Color::Rgb(26, 42, 26);

/// Diff deleted line foreground (red text).
pub(crate) const DIFF_DEL_FG: Color = Color::Rgb(247, 118, 142);

/// Diff deleted line background (dark red).
pub(crate) const DIFF_DEL_BG: Color = Color::Rgb(42, 26, 26);

// ── Message roles ────────────────────────────────────────────────────

/// User message prompt marker color.
pub(crate) const USER_MARKER: Color = Color::Rgb(125, 207, 255);

/// User message background tint.
pub(crate) const USER_BG: Color = Color::Rgb(25, 45, 80);

/// Reasoning / thinking text color (dimmed blue-gray).
pub(crate) const REASONING: Color = Color::Rgb(130, 140, 170);

/// Hidden reasoning hint text.
pub(crate) const REASONING_HIDDEN: Color = Color::Rgb(105, 110, 130);

// ── Activity / live feed ─────────────────────────────────────────────

/// Activity warning text (dim orange-yellow).
pub(crate) const ACTIVITY_WARN: Color = Color::Rgb(180, 150, 60);

/// Activity failure text (dim red).
pub(crate) const ACTIVITY_FAIL: Color = Color::Rgb(190, 70, 70);

/// Pending tool header text (light gray-blue).
pub(crate) const ACTIVITY_PENDING: Color = Color::Rgb(120, 125, 145);

/// Completed tool header text (dimmed).
pub(crate) const ACTIVITY_DONE: Color = Color::Rgb(160, 165, 180);

// ── Token / context usage thresholds ─────────────────────────────────

/// Token count text color.
pub(crate) const TOKENS: Color = Color::Rgb(158, 206, 106);

/// Context usage — low (<50%).
pub(crate) const CTX_LOW: Color = Color::Rgb(158, 206, 106);

/// Context usage — medium (50-75%).
pub(crate) const CTX_MED: Color = Color::Rgb(224, 175, 104);

/// Context usage — high (>75%).
pub(crate) const CTX_HIGH: Color = Color::Rgb(247, 118, 142);

// ── Cursor ──────────────────────────────────────────────────────────

/// Block cursor foreground (dark background color for contrast).
pub(crate) const CURSOR_FG: Color = Color::Rgb(30, 30, 46);

/// Block cursor background (light gray block).
pub(crate) const CURSOR_BG: Color = Color::Rgb(192, 202, 245);

/// Render a block cursor at `(x, y)` in the frame buffer.
///
/// Styles the cell at the given position with [`CURSOR_FG`] / [`CURSOR_BG`]
/// to produce a visible block cursor.  Bounds-checks against `area` so
/// callers don't need to.
pub(crate) fn render_block_cursor(
    frame: &mut ratatui::Frame<'_>,
    x: u16,
    y: u16,
    area: ratatui::layout::Rect,
) {
    use ratatui::style::Style;

    if x < area.x + area.width && y < area.y + area.height {
        let buf = frame.buffer_mut();
        let cell = &mut buf[(x, y)];
        cell.set_style(Style::default().fg(CURSOR_FG).bg(CURSOR_BG));
    }
}
