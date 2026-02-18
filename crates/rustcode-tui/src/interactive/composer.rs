use super::ChatState;

fn composer_set(chat: &mut ChatState, text: String) {
    chat.composer = text;
    chat.composer_cursor = chat.composer.len();
}

pub(super) fn composer_clear(chat: &mut ChatState) {
    chat.composer.clear();
    chat.composer_cursor = 0;
}

pub(super) fn composer_insert_str(chat: &mut ChatState, s: &str) {
    let idx = chat.composer_cursor.min(chat.composer.len());
    chat.composer.insert_str(idx, s);
    chat.composer_cursor = (idx + s.len()).min(chat.composer.len());
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
            col += 1;
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
        col += 1;
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
            cur_col += 1;
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

pub(super) fn composer_cursor_visual(text: &str, cursor: usize, width: u16) -> (usize, usize) {
    let width = width.max(1) as usize;
    let cursor = cursor.min(text.len());
    let mut row = 0usize;
    let mut col = 0usize;
    let mut byte = 0usize;
    for ch in text.chars() {
        if byte >= cursor {
            break;
        }
        if ch == '\n' {
            row += 1;
            col = 0;
            byte += 1;
            continue;
        }
        col += 1;
        byte += ch.len_utf8();
        if col >= width {
            row += 1;
            col = 0;
        }
    }
    (row, col)
}

pub(super) fn history_prev(chat: &mut ChatState) {
    if chat.prompt_history.is_empty() {
        return;
    }
    let next = match chat.history_cursor {
        None => {
            chat.history_draft = chat.composer.clone();
            chat.prompt_history.len().saturating_sub(1)
        }
        Some(0) => 0,
        Some(idx) => idx.saturating_sub(1),
    };
    chat.history_cursor = Some(next);
    if let Some(value) = chat.prompt_history.get(next) {
        composer_set(chat, value.clone());
    }
}

pub(super) fn history_next(chat: &mut ChatState) {
    let Some(idx) = chat.history_cursor else {
        return;
    };
    if idx + 1 >= chat.prompt_history.len() {
        chat.history_cursor = None;
        composer_set(chat, chat.history_draft.clone());
        return;
    }
    let next = idx + 1;
    chat.history_cursor = Some(next);
    if let Some(value) = chat.prompt_history.get(next) {
        composer_set(chat, value.clone());
    }
}
