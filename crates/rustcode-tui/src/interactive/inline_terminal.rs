//! Inline terminal viewport for rendering a growing TUI without alternate screen.
//!
//! Instead of taking over the entire screen, the TUI renders inline starting at
//! the cursor position and grows upward as content increases (like Claude Code
//! and Codex). Previous terminal output remains visible above the viewport.
//!
//! Uses a `TestBackend`-backed ratatui `Terminal` for off-screen rendering so
//! that all existing render functions (which accept `ratatui::Frame`) work
//! unchanged. The rendered buffer is diffed against the previous frame and
//! flushed to real stdout at the correct viewport position.

use std::io::{self, Stdout, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{
    Attribute, Colors, Print, SetAttribute, SetBackgroundColor, SetColors, SetForegroundColor,
};
use crossterm::terminal::Clear;
use ratatui::backend::{Backend, TestBackend};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

use super::TuiError;

/// Inline terminal that renders into a viewport at a fixed position in the
/// real terminal, growing upward as content demands more height.
pub(super) struct InlineTerminal {
    /// Real stdout handle for flushing draw commands.
    stdout: Stdout,
    /// Off-screen ratatui terminal used to produce `Frame` for render functions.
    render_terminal: Terminal<TestBackend>,
    /// Previous frame buffer for diffing.
    prev_buffer: Buffer,
    /// Current viewport rectangle in real terminal coordinates.
    viewport: Rect,
    /// Last known real terminal dimensions (cols, rows).
    screen_width: u16,
    screen_height: u16,
    /// Whether the cursor is currently hidden on the real terminal.
    cursor_hidden: bool,
    /// Cursor position captured from the last draw call.
    /// `None` means cursor was hidden (no `set_cursor_position` called).
    last_cursor: Option<Position>,
}

impl InlineTerminal {
    /// Create a new inline terminal at the current cursor position.
    ///
    /// # Errors
    /// Returns `TuiError` if terminal size or cursor position cannot be queried.
    pub(super) fn new() -> Result<Self, TuiError> {
        let (cols, rows) = crossterm::terminal::size().map_err(|e| TuiError::Io(e.to_string()))?;
        let cursor_y = crossterm::cursor::position()
            .map(|(_, y)| y)
            .unwrap_or(rows.saturating_sub(1));

        let backend = TestBackend::new(1, 1);
        let render_terminal = Terminal::new(backend).map_err(|e| TuiError::Io(e.to_string()))?;

        Ok(Self {
            stdout: io::stdout(),
            render_terminal,
            prev_buffer: Buffer::empty(Rect::ZERO),
            viewport: Rect::new(0, cursor_y, cols, 0),
            screen_width: cols,
            screen_height: rows,
            cursor_hidden: false,
            last_cursor: None,
        })
    }

    /// Return the real terminal size as `(width, height)`.
    pub(super) fn size() -> Result<(u16, u16), TuiError> {
        crossterm::terminal::size().map_err(|e| TuiError::Io(e.to_string()))
    }

    /// Draw one frame with the given desired height.
    ///
    /// The render function receives a standard `ratatui::Frame` so all existing
    /// render code works unchanged.
    ///
    /// # Errors
    /// Returns `TuiError` on I/O failure.
    pub(super) fn draw<F>(&mut self, desired_height: u16, render_fn: F) -> Result<(), TuiError>
    where
        F: FnOnce(&mut ratatui::Frame<'_>),
    {
        // Detect terminal resize.
        let prev_screen_w = self.screen_width;
        let prev_screen_h = self.screen_height;
        if let Ok((w, h)) = crossterm::terminal::size() {
            self.screen_width = w;
            self.screen_height = h;
        }
        let screen_resized =
            self.screen_width != prev_screen_w || self.screen_height != prev_screen_h;

        let height = desired_height.min(self.screen_height).max(1);
        let width = self.screen_width;

        // Compute new viewport position.
        let mut new_y = self.viewport.y;

        // When the terminal grows back after being shrunk, the emulator may
        // pull old content from scrollback into the newly visible rows.
        // Reanchor the viewport to the bottom of the screen to avoid ghost
        // content appearing below our viewport.
        if screen_resized {
            new_y = self.screen_height.saturating_sub(height);
        }

        let new_bottom = new_y.saturating_add(height);

        if new_bottom > self.screen_height {
            // Need to scroll content above the viewport upward to make room.
            let scroll_by = new_bottom - self.screen_height;
            if new_y > 0 {
                let scroll_amount = scroll_by.min(new_y);
                self.scroll_region_up(0, new_y, scroll_amount)
                    .map_err(|e| TuiError::Io(e.to_string()))?;
            }
            new_y = self.screen_height.saturating_sub(height);
        }

        let new_viewport = Rect::new(0, new_y, width, height);
        let old_viewport = self.viewport;
        let viewport_changed = new_viewport != old_viewport;
        self.viewport = new_viewport;

        // Resize the off-screen terminal if dimensions changed.
        let current_size = self
            .render_terminal
            .size()
            .map_err(|e| TuiError::Io(e.to_string()))?;
        if current_size.width != width || current_size.height != height {
            self.render_terminal.backend_mut().resize(width, height);
        }

        if screen_resized {
            // Terminal size changed — clear the ENTIRE screen to wipe ghost
            // content that may have been pulled from scrollback or left behind
            // when the terminal shrank then grew.
            self.clear_region(0, self.screen_height)
                .map_err(|e| TuiError::Io(e.to_string()))?;
        } else if viewport_changed {
            // Clear from the topmost extent to the bottommost extent of
            // both old and new viewports to eliminate all ghost lines.
            let clear_top = old_viewport.y.min(new_viewport.y);
            let clear_bottom = (old_viewport.y + old_viewport.height)
                .max(new_viewport.y + new_viewport.height)
                .min(self.screen_height);
            self.clear_region(clear_top, clear_bottom)
                .map_err(|e| TuiError::Io(e.to_string()))?;
        }

        // Always clear previous buffer to force a full redraw every frame.
        // This eliminates ghosting from buffer state mismatches between
        // ratatui's internal Terminal and our viewport management.
        self.prev_buffer = Buffer::empty(Rect::ZERO);

        // Before draw, move backend cursor to a sentinel position so we can
        // detect whether the render code called set_cursor_position.
        let sentinel = Position {
            x: width.saturating_sub(1),
            y: height,
        };
        let _ = self
            .render_terminal
            .backend_mut()
            .set_cursor_position(sentinel);

        // Render into the off-screen terminal.
        self.render_terminal
            .draw(|frame| {
                render_fn(frame);
            })
            .map_err(|e| TuiError::Io(e.to_string()))?;

        // After draw, check if cursor was repositioned from the sentinel.
        // If ratatui's Terminal called set_cursor_position (cursor visible),
        // the position will differ from the sentinel.
        // If ratatui called hide_cursor (no cursor set in frame), the position
        // stays at the sentinel.
        let backend_pos = self
            .render_terminal
            .backend_mut()
            .get_cursor_position()
            .ok();
        let cursor_pos = backend_pos.filter(|p| *p != sentinel);
        self.last_cursor = cursor_pos;

        // Get the rendered buffer and diff against the previous frame.
        let current_buffer = self.render_terminal.backend().buffer().clone();

        self.flush_diff(&current_buffer)
            .map_err(|e| TuiError::Io(e.to_string()))?;

        // Handle cursor visibility and position on the real terminal.
        // ratatui's Terminal calls show_cursor+set_cursor_position when frame
        // had a cursor, or hide_cursor when it didn't. We read the position
        // from the TestBackend and always show/position our real cursor if the
        // render code set a cursor position.
        if let Some(pos) = cursor_pos {
            let real_x = pos.x;
            let real_y = self.viewport.y + pos.y;
            queue!(self.stdout, Show, MoveTo(real_x, real_y))
                .map_err(|e| TuiError::Io(e.to_string()))?;
            self.cursor_hidden = false;
        } else if !self.cursor_hidden {
            queue!(self.stdout, Hide).map_err(|e| TuiError::Io(e.to_string()))?;
            self.cursor_hidden = true;
        }

        self.stdout
            .flush()
            .map_err(|e| TuiError::Io(e.to_string()))?;

        self.prev_buffer = current_buffer;
        Ok(())
    }

    /// Clean up the terminal on exit: clear viewport, move cursor to viewport
    /// top, show cursor, and reset scroll region.
    pub(super) fn cleanup(&mut self) {
        // Clear every line the viewport occupied so no ghost content remains.
        let _ = queue!(
            self.stdout,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(crossterm::style::Color::Reset),
            SetBackgroundColor(crossterm::style::Color::Reset),
        );
        for y in self.viewport.y..self.viewport.y.saturating_add(self.viewport.height) {
            let _ = queue!(
                self.stdout,
                MoveTo(0, y),
                Clear(crossterm::terminal::ClearType::CurrentLine)
            );
        }
        let _ = queue!(self.stdout, Show, MoveTo(0, self.viewport.y));
        // Reset scroll region and add a newline for clean shell prompt.
        let _ = writeln!(self.stdout, "\x1b[r");
        let _ = self.stdout.flush();
        self.cursor_hidden = false;
    }

    /// Scroll a region of the terminal upward using ANSI escape sequences.
    ///
    /// Scrolls lines in `[0..region_bottom)` up by `scroll_by` lines,
    /// effectively pushing content above the viewport off-screen.
    fn scroll_region_up(
        &mut self,
        _region_top: u16,
        region_bottom: u16,
        scroll_by: u16,
    ) -> io::Result<()> {
        if scroll_by == 0 || region_bottom == 0 {
            return Ok(());
        }
        // Set scroll region to [1..region_bottom] (1-indexed for ANSI).
        write!(self.stdout, "\x1b[1;{region_bottom}r")?;
        // Scroll up within the region.
        write!(self.stdout, "\x1b[{scroll_by}S")?;
        // Reset scroll region to full screen.
        write!(self.stdout, "\x1b[r")?;
        self.stdout.flush()
    }

    /// Clear a region of the real terminal from `top_y` (inclusive) to
    /// `bottom_y` (exclusive).
    fn clear_region(&mut self, top_y: u16, bottom_y: u16) -> io::Result<()> {
        for y in top_y..bottom_y {
            queue!(
                self.stdout,
                MoveTo(0, y),
                Clear(crossterm::terminal::ClearType::CurrentLine)
            )?;
        }
        self.stdout.flush()
    }

    /// Diff the current buffer against the previous and flush changes to stdout.
    fn flush_diff(&mut self, current: &Buffer) -> io::Result<()> {
        let commands = diff_buffers(&self.prev_buffer, current, self.viewport);
        draw_commands(&mut self.stdout, commands.into_iter())
    }
}

impl Drop for InlineTerminal {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ── Buffer diffing (ported from Codex custom_terminal.rs) ─────────────────

enum DrawCommand {
    Put { x: u16, y: u16, cell: Cell },
    ClearToEnd { x: u16, y: u16, bg: Color },
}

impl DrawCommand {
    #[cfg(test)]
    #[must_use]
    const fn is_put(&self) -> bool {
        matches!(self, Self::Put { .. })
    }
}

/// Diff two buffers and produce a list of draw commands.
///
/// `viewport` provides the offset so that x/y coordinates are in real terminal
/// space (not buffer-local).
fn diff_buffers(prev: &Buffer, next: &Buffer, viewport: Rect) -> Vec<DrawCommand> {
    let area = next.area;
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let next_content = next.content();
    let mut commands = Vec::new();
    let mut last_nonblank_cols = vec![0u16; area.height as usize];

    // First pass: find rightmost non-blank column per row and emit ClearToEnd.
    for row in 0..area.height {
        let row_start = row as usize * area.width as usize;
        let row_end = row_start + area.width as usize;
        let row_cells = &next_content[row_start..row_end];
        let bg = row_cells.last().map_or(Color::Reset, |c| c.bg);

        let mut last_nonblank: u16 = 0;
        let mut col = 0usize;
        while col < row_cells.len() {
            let cell = &row_cells[col];
            let w = cell.symbol().width();
            if cell.symbol() != " " || cell.bg != bg || cell.modifier != Modifier::empty() {
                last_nonblank = (col + w.saturating_sub(1)) as u16;
            }
            col += w.max(1);
        }

        if (last_nonblank as usize + 1) < row_cells.len() {
            commands.push(DrawCommand::ClearToEnd {
                x: viewport.x + last_nonblank + 1,
                y: viewport.y + row,
                bg,
            });
        }

        last_nonblank_cols[row as usize] = last_nonblank;
    }

    // Second pass: emit Put commands for changed cells.
    let prev_content = prev.content();
    let prev_width = if prev.area.width > 0 {
        prev.area.width
    } else {
        0
    };
    let prev_height = prev.area.height;

    let mut invalidated: usize = 0;
    let mut to_skip: usize = 0;

    for (i, cell) in next_content.iter().enumerate() {
        let bx = (i % area.width as usize) as u16;
        let by = (i / area.width as usize) as u16;

        // Look up the corresponding previous cell (if it exists).
        let prev_cell = if prev_width > 0 && bx < prev_width && by < prev_height {
            let prev_idx = by as usize * prev_width as usize + bx as usize;
            prev_content.get(prev_idx)
        } else {
            None
        };

        let changed = prev_cell != Some(cell);

        if !cell.skip
            && (changed || invalidated > 0)
            && to_skip == 0
            && bx <= last_nonblank_cols[by as usize]
        {
            commands.push(DrawCommand::Put {
                x: viewport.x + bx,
                y: viewport.y + by,
                cell: cell.clone(),
            });
        }

        to_skip = cell.symbol().width().saturating_sub(1);

        let prev_w = prev_cell.map_or(1, |p| p.symbol().width());
        let cur_w = cell.symbol().width();
        invalidated = invalidated.max(cur_w).max(prev_w).saturating_sub(1);
    }

    commands
}

/// Write draw commands to a writer using crossterm queue.
fn draw_commands<I>(writer: &mut impl Write, commands: I) -> io::Result<()>
where
    I: Iterator<Item = DrawCommand>,
{
    let mut fg = Color::Reset;
    let mut bg = Color::Reset;
    let mut modifier = Modifier::empty();
    let mut last_pos: Option<Position> = None;

    for command in commands {
        let (x, y) = match &command {
            DrawCommand::Put { x, y, .. } | DrawCommand::ClearToEnd { x, y, .. } => (*x, *y),
        };

        // Only emit MoveTo if cursor isn't already at the right position.
        if !matches!(last_pos, Some(p) if x == p.x && y == p.y) {
            queue!(writer, MoveTo(x, y))?;
        }

        match command {
            DrawCommand::Put { cell, .. } => {
                let sym_width = cell.symbol().width().max(1) as u16;
                if cell.modifier != modifier {
                    queue_modifier_diff(writer, modifier, cell.modifier)?;
                    modifier = cell.modifier;
                }
                if cell.fg != fg || cell.bg != bg {
                    queue!(
                        writer,
                        SetColors(Colors::new(cell.fg.into(), cell.bg.into()))
                    )?;
                    fg = cell.fg;
                    bg = cell.bg;
                }
                queue!(writer, Print(cell.symbol()))?;
                last_pos = Some(Position {
                    x: x + sym_width,
                    y,
                });
            }
            DrawCommand::ClearToEnd { bg: clear_bg, .. } => {
                queue!(writer, SetAttribute(Attribute::Reset))?;
                modifier = Modifier::empty();
                queue!(
                    writer,
                    SetBackgroundColor(clear_bg.into()),
                    Clear(crossterm::terminal::ClearType::UntilNewLine)
                )?;
                bg = clear_bg;
                last_pos = None; // cursor position unknown after clear
            }
        }
    }

    // Reset styles at the end of the frame.
    queue!(
        writer,
        SetForegroundColor(crossterm::style::Color::Reset),
        SetBackgroundColor(crossterm::style::Color::Reset),
        SetAttribute(Attribute::Reset),
    )?;

    Ok(())
}

/// Queue modifier diff commands (add/remove bold, italic, etc.).
fn queue_modifier_diff(writer: &mut impl Write, from: Modifier, to: Modifier) -> io::Result<()> {
    let removed = from - to;
    if removed.contains(Modifier::REVERSED) {
        queue!(writer, SetAttribute(Attribute::NoReverse))?;
    }
    if removed.contains(Modifier::BOLD) {
        queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
        if to.contains(Modifier::DIM) {
            queue!(writer, SetAttribute(Attribute::Dim))?;
        }
    }
    if removed.contains(Modifier::ITALIC) {
        queue!(writer, SetAttribute(Attribute::NoItalic))?;
    }
    if removed.contains(Modifier::UNDERLINED) {
        queue!(writer, SetAttribute(Attribute::NoUnderline))?;
    }
    if removed.contains(Modifier::DIM) {
        queue!(writer, SetAttribute(Attribute::NormalIntensity))?;
    }
    if removed.contains(Modifier::CROSSED_OUT) {
        queue!(writer, SetAttribute(Attribute::NotCrossedOut))?;
    }
    if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
        queue!(writer, SetAttribute(Attribute::NoBlink))?;
    }

    let added = to - from;
    if added.contains(Modifier::REVERSED) {
        queue!(writer, SetAttribute(Attribute::Reverse))?;
    }
    if added.contains(Modifier::BOLD) {
        queue!(writer, SetAttribute(Attribute::Bold))?;
    }
    if added.contains(Modifier::ITALIC) {
        queue!(writer, SetAttribute(Attribute::Italic))?;
    }
    if added.contains(Modifier::UNDERLINED) {
        queue!(writer, SetAttribute(Attribute::Underlined))?;
    }
    if added.contains(Modifier::DIM) {
        queue!(writer, SetAttribute(Attribute::Dim))?;
    }
    if added.contains(Modifier::CROSSED_OUT) {
        queue!(writer, SetAttribute(Attribute::CrossedOut))?;
    }
    if added.contains(Modifier::SLOW_BLINK) {
        queue!(writer, SetAttribute(Attribute::SlowBlink))?;
    }
    if added.contains(Modifier::RAPID_BLINK) {
        queue!(writer, SetAttribute(Attribute::RapidBlink))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_buffers_empty_prev() {
        let viewport = Rect::new(0, 5, 10, 3);
        let prev = Buffer::empty(Rect::ZERO);
        let mut next = Buffer::empty(Rect::new(0, 0, 10, 3));
        next.set_string(0, 0, "hello", ratatui::style::Style::default());

        let commands = diff_buffers(&prev, &next, viewport);
        // Should have Put commands for the changed cells.
        let put_count = commands.iter().filter(|c| c.is_put()).count();
        assert!(put_count > 0, "expected Put commands for new content");
    }

    #[test]
    fn diff_buffers_no_change() {
        let viewport = Rect::new(0, 0, 5, 2);
        let buf = Buffer::empty(Rect::new(0, 0, 5, 2));

        let commands = diff_buffers(&buf, &buf, viewport);
        let put_count = commands.iter().filter(|c| c.is_put()).count();
        assert_eq!(
            put_count, 0,
            "identical buffers should produce no Put commands"
        );
    }

    #[test]
    fn diff_buffers_viewport_offset() {
        let viewport = Rect::new(0, 10, 5, 1);
        let prev = Buffer::empty(Rect::ZERO);
        let mut next = Buffer::empty(Rect::new(0, 0, 5, 1));
        next.set_string(0, 0, "X", ratatui::style::Style::default());

        let commands = diff_buffers(&prev, &next, viewport);
        // The Put command should have y = 10 (viewport offset).
        let put = commands.iter().find(|c| c.is_put());
        assert!(put.is_some());
        if let Some(DrawCommand::Put { y, .. }) = put {
            assert_eq!(*y, 10, "Put y should include viewport offset");
        }
    }
}
