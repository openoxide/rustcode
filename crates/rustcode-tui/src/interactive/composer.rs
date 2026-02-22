use std::time::Instant;

use unicode_width::UnicodeWidthChar;

use super::{AppState, ChatState};

fn char_display_width(ch: char) -> usize {
    if ch == '\t' {
        4
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}

fn composer_set(chat: &mut ChatState, text: String) {
    chat.composer = text;
    chat.composer_cursor = chat.composer.len();
    chat.last_typing_time = Some(Instant::now());
    // Reset the Ctrl+C cleared flag since user is typing new content
    chat.composer_cleared_by_ctrl_c = false;
}

pub(super) fn composer_clear(chat: &mut ChatState) {
    chat.composer.clear();
    chat.composer_cursor = 0;
}

pub(super) fn composer_insert_str(chat: &mut ChatState, s: &str) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    chat.composer.insert_str(idx, s);
    chat.composer_cursor = (idx + s.len()).min(chat.composer.len());
    // Track typing activity for the indicator
    chat.last_typing_time = Some(Instant::now());
    // Reset the Ctrl+C cleared flag since user is typing new content
    chat.composer_cleared_by_ctrl_c = false;
}

pub(super) fn composer_move_left(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    let mut prev = 0;
    for (i, _) in chat.composer.char_indices() {
        if i >= idx {
            break;
        }
        prev = i;
    }
    if idx == 0 {
        chat.composer_cursor = 0;
    } else {
        chat.composer_cursor = prev;
    }
}

pub(super) fn composer_move_right(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    if idx >= chat.composer.len() {
        chat.composer_cursor = chat.composer.len();
        return;
    }
    if let Some((i, ch)) = chat.composer[idx..].char_indices().next() {
        chat.composer_cursor = (idx + i + ch.len_utf8()).min(chat.composer.len());
    }
}

pub(super) fn composer_backspace(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    if idx == 0 {
        return;
    }
    let mut prev = 0;
    for (i, _) in chat.composer.char_indices() {
        if i >= idx {
            break;
        }
        prev = i;
    }
    chat.composer.replace_range(prev..idx, "");
    chat.composer_cursor = prev;
    // Track typing activity
    chat.last_typing_time = Some(Instant::now());
}

pub(super) fn composer_delete(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    if idx >= chat.composer.len() {
        return;
    }
    if let Some((i, ch)) = chat.composer[idx..].char_indices().next() {
        let next = idx + i + ch.len_utf8();
        if next > idx {
            chat.composer.replace_range(idx..next, "");
        }
    }
    // Track typing activity
    chat.last_typing_time = Some(Instant::now());
}

fn composer_line_col(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(text.len());
    let mut line = 0usize;
    let mut col = 0usize;
    let mut i = 0usize;
    for ch in text.chars() {
        if i >= cursor {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += char_display_width(ch);
        }
        i += ch.len_utf8();
    }
    (line, col)
}

fn composer_cursor_from_line_col(text: &str, target_line: usize, target_col: usize) -> usize {
    let mut line = 0usize;
    let mut col = 0usize;
    let mut byte = 0usize;
    for ch in text.chars() {
        if line == target_line && col >= target_col {
            break;
        }
        if ch == '\n' {
            if line == target_line {
                break;
            }
            line += 1;
            col = 0;
            byte += 1;
            continue;
        }
        col += char_display_width(ch);
        byte += ch.len_utf8();
    }
    byte.min(text.len())
}

pub(super) fn composer_move_home(chat: &mut ChatState) {
    let (line, _) = composer_line_col(&chat.composer, chat.composer_cursor);
    chat.composer_cursor = composer_cursor_from_line_col(&chat.composer, line, 0);
}

pub(super) fn composer_move_end(chat: &mut ChatState) {
    let (line, _) = composer_line_col(&chat.composer, chat.composer_cursor);
    let mut max_col = 0usize;
    let mut cur_line = 0usize;
    let mut cur_col = 0usize;
    for ch in chat.composer.chars() {
        if ch == '\n' {
            if cur_line == line {
                break;
            }
            cur_line += 1;
            cur_col = 0;
            continue;
        }
        if cur_line == line {
            cur_col += char_display_width(ch);
            max_col = cur_col;
        }
    }
    chat.composer_cursor = composer_cursor_from_line_col(&chat.composer, line, max_col);
}

pub(super) fn composer_move_up(chat: &mut ChatState) {
    let (line, col) = composer_line_col(&chat.composer, chat.composer_cursor);
    if line == 0 {
        return;
    }
    chat.composer_cursor = composer_cursor_from_line_col(&chat.composer, line - 1, col);
}

pub(super) fn composer_move_down(chat: &mut ChatState) {
    let (line, col) = composer_line_col(&chat.composer, chat.composer_cursor);
    let lines = chat.composer.chars().filter(|ch| *ch == '\n').count() + 1;
    if line + 1 >= lines {
        return;
    }
    chat.composer_cursor = composer_cursor_from_line_col(&chat.composer, line + 1, col);
}

/// Compute the visual (row, col) of the cursor in the composer text, using
/// **word wrapping** that matches `char_wrap_text` and
/// `composer_wrapped_line_count`.
///
/// Words that are wider than `width` fall back to character wrapping.
pub(super) fn composer_cursor_visual(text: &str, cursor: usize, width: u16) -> (usize, usize) {
    let width = width.max(1) as usize;
    let cursor = cursor.min(text.len());
    let mut row = 0usize;
    let mut col = 0usize;
    let mut byte = 0usize;

    // Iterate over "segments": sequences of non-space chars (words) and
    // individual space/newline chars.  Before placing a word, check if it
    // fits on the current line; if not, wrap the whole word to the next line.
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];

        // Check cursor before processing this character.
        if byte >= cursor {
            break;
        }

        if ch == '\n' {
            row += 1;
            col = 0;
            byte += ch.len_utf8();
            i += 1;
            continue;
        }

        // Space: place it and advance.
        if ch == ' ' || ch == '\t' {
            let cw = char_display_width(ch);
            col += cw;
            byte += ch.len_utf8();
            if col >= width {
                row += 1;
                col = 0;
            }
            i += 1;
            continue;
        }

        // Start of a word — measure its full width.
        let word_start = i;
        let mut word_w = 0usize;
        let mut j = i;
        while j < chars.len() && chars[j] != ' ' && chars[j] != '\t' && chars[j] != '\n' {
            word_w += char_display_width(chars[j]);
            j += 1;
        }

        // If the word fits in the line width but not at current col, wrap first.
        if word_w <= width && col > 0 && col + word_w > width {
            row += 1;
            col = 0;
        }

        // Place the word character by character (handles words wider than width).
        for ch in chars.iter().take(j).skip(word_start) {
            if byte >= cursor {
                break;
            }
            let cw = char_display_width(*ch);
            // Character-wrap for oversized words.
            if col > 0 && col + cw > width {
                row += 1;
                col = 0;
            }
            col += cw;
            byte += ch.len_utf8();
        }
        i = j;
    }
    (row, col)
}

/// Jump cursor left past any trailing whitespace, then past the preceding word.
pub(super) fn composer_word_left(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    let before = &chat.composer[..idx];
    // skip trailing whitespace, then find the last whitespace boundary
    let trimmed = before.trim_end();
    let new_idx = trimmed.rfind(|c: char| c.is_whitespace()).map_or(0, |p| {
        // p is the byte index of the whitespace char; we want the next char start
        let ch = trimmed[p..].chars().next().map_or(1, |c| c.len_utf8());
        p + ch
    });
    chat.composer_cursor = new_idx;
}

/// Jump cursor right past the current word, then past any following whitespace.
pub(super) fn composer_word_right(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    let after = &chat.composer[idx..];
    // skip non-whitespace (the word), then skip whitespace
    let after_word = after.trim_start_matches(|c: char| !c.is_whitespace());
    let after_space = after_word.trim_start();
    let delta = after.len() - after_space.len();
    chat.composer_cursor = (idx + delta).min(chat.composer.len());
}

/// Delete from cursor to end of the current line (does not delete the `\n`).
pub(super) fn composer_kill_line_forward(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    let end = chat.composer[idx..]
        .find('\n')
        .map_or(chat.composer.len(), |p| idx + p);
    if end > idx {
        chat.composer.replace_range(idx..end, "");
    }
    chat.last_typing_time = Some(std::time::Instant::now());
}

/// Delete from the start of the current line to the cursor.
pub(super) fn composer_kill_line_backward(chat: &mut ChatState) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    let start = chat.composer[..idx].rfind('\n').map_or(0, |p| p + 1);
    if idx > start {
        chat.composer.replace_range(start..idx, "");
        chat.composer_cursor = start;
    }
    chat.last_typing_time = Some(std::time::Instant::now());
}

pub(super) fn history_prev(state: &mut AppState, chat: &mut ChatState) {
    if state.global_prompt_history.is_empty() {
        return;
    }
    let next = match state.history_cursor {
        None => {
            state.history_draft = chat.composer.clone();
            state.global_prompt_history.len().saturating_sub(1)
        }
        Some(0) => 0,
        Some(idx) => idx.saturating_sub(1),
    };
    state.history_cursor = Some(next);
    if let Some(value) = state.global_prompt_history.get(next) {
        composer_set(chat, value.clone());
    }
}

pub(super) fn history_next(state: &mut AppState, chat: &mut ChatState) {
    let Some(idx) = state.history_cursor else {
        return;
    };
    if idx + 1 >= state.global_prompt_history.len() {
        state.history_cursor = None;
        composer_set(chat, state.history_draft.clone());
        return;
    }
    let next = idx + 1;
    state.history_cursor = Some(next);
    if let Some(value) = state.global_prompt_history.get(next) {
        composer_set(chat, value.clone());
    }
}

/// Word-wrap `text` into lines of at most `width` display columns.
///
/// Words that fit within `width` but would overflow the current line are
/// moved to the next line as a whole.  Words wider than `width` fall back
/// to character wrapping.  The wrapping logic matches
/// [`composer_cursor_visual`] so the cursor and the rendered text agree.
pub(super) fn word_wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if ch == '\n' {
            lines.push(std::mem::take(&mut current));
            col = 0;
            i += 1;
            continue;
        }

        // Space / tab — place directly.
        if ch == ' ' || ch == '\t' {
            let cw = ch.width().unwrap_or(1);
            col += cw;
            current.push(ch);
            if col >= width {
                lines.push(std::mem::take(&mut current));
                col = 0;
            }
            i += 1;
            continue;
        }

        // Start of a word — measure full width.
        let word_start = i;
        let mut word_w = 0usize;
        let mut j = i;
        while j < chars.len() && chars[j] != ' ' && chars[j] != '\t' && chars[j] != '\n' {
            word_w += chars[j].width().unwrap_or(1);
            j += 1;
        }

        // If the word fits within line width but not at current col, wrap first.
        if word_w <= width && col > 0 && col + word_w > width {
            lines.push(std::mem::take(&mut current));
            col = 0;
        }

        // Place word characters (character-wrap for oversized words).
        for ch in chars.iter().take(j).skip(word_start) {
            let cw = ch.width().unwrap_or(1);
            if col > 0 && col + cw > width {
                lines.push(std::mem::take(&mut current));
                col = 0;
            }
            current.push(*ch);
            col += cw;
        }
        i = j;
    }
    lines.push(current);
    lines
}
