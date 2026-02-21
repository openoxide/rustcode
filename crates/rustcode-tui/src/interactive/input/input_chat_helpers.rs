use super::{filter_slash_commands, AppState, ChatState, Modal};

/// Replace the composer content with the currently selected slash command and
/// close the [`Modal::SlashHelp`] popup.
///
/// For commands that take arguments (contain a space in the table entry, e.g.
/// `/find <query>`), the composer is populated with `/cmd ` (trailing space)
/// so the user can type the argument immediately.  For argument-free commands
/// the full command text is inserted.
pub(super) fn complete_slash_selection(state: &mut AppState, chat: &mut ChatState) {
    let completion = if let Some(Modal::SlashHelp { query, selected }) = &state.modal {
        let filtered = filter_slash_commands(query);
        filtered.get(*selected).map(|(cmd, _)| {
            // Strip the leading `/`, split on whitespace to get the bare name.
            let bare = cmd.trim_start_matches('/');
            if bare.contains(' ') {
                // Command takes arguments — insert `/cmd ` with trailing space.
                let stem = bare.split_whitespace().next().unwrap_or(bare);
                format!("/{stem} ")
            } else {
                format!("/{bare}")
            }
        })
    } else {
        None
    };

    if let Some(text) = completion {
        // Replace composer with the completed command.
        let byte_len = text.len();
        chat.composer = text;
        chat.composer_cursor = byte_len;
        state.modal = None;
    }
}

pub(super) fn consume_large_paste_summary_with_backspace(
    composer: &mut String,
    cursor: &mut usize,
    paste_buffer: &mut Option<String>,
) -> bool {
    let Some(full_text) = paste_buffer.as_deref() else {
        return false;
    };
    let Some((summary_start, summary_end)) = large_paste_summary_span(composer, full_text) else {
        return false;
    };
    let idx = (*cursor).min(composer.len());
    if idx <= summary_start || idx > summary_end {
        return false;
    }
    composer.replace_range(summary_start..summary_end, "");
    *cursor = summary_start;
    *paste_buffer = None;
    true
}

pub(super) fn consume_large_paste_summary_with_delete(
    composer: &mut String,
    cursor: &mut usize,
    paste_buffer: &mut Option<String>,
) -> bool {
    let Some(full_text) = paste_buffer.as_deref() else {
        return false;
    };
    let Some((summary_start, summary_end)) = large_paste_summary_span(composer, full_text) else {
        return false;
    };
    let idx = (*cursor).min(composer.len());
    if idx < summary_start || idx >= summary_end {
        return false;
    }
    composer.replace_range(summary_start..summary_end, "");
    *cursor = summary_start;
    *paste_buffer = None;
    true
}

fn large_paste_summary_span(displayed: &str, full_text: &str) -> Option<(usize, usize)> {
    if displayed == full_text {
        return None;
    }

    let prefix = common_prefix_len(displayed, full_text);
    let suffix = common_suffix_len(displayed, full_text, prefix);

    let display_mid_start = prefix;
    let display_mid_end = displayed.len().saturating_sub(suffix);
    if display_mid_start >= display_mid_end {
        return None;
    }
    Some((display_mid_start, display_mid_end))
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut a_iter = a.char_indices();
    let mut b_iter = b.char_indices();
    let mut prefix = 0usize;
    loop {
        match (a_iter.next(), b_iter.next()) {
            (Some((ai, ac)), Some((_, bc))) if ac == bc => {
                prefix = ai + ac.len_utf8();
            }
            _ => break,
        }
    }
    prefix
}

fn common_suffix_len(a: &str, b: &str, prefix: usize) -> usize {
    let a_tail = &a[prefix.min(a.len())..];
    let b_tail = &b[prefix.min(b.len())..];
    let mut a_iter = a_tail.chars().rev();
    let mut b_iter = b_tail.chars().rev();
    let mut suffix = 0usize;
    loop {
        match (a_iter.next(), b_iter.next()) {
            (Some(ac), Some(bc)) if ac == bc => {
                suffix += ac.len_utf8();
            }
            _ => break,
        }
    }
    suffix
}

#[cfg(test)]
mod tests {
    use super::consume_large_paste_summary_with_backspace;

    #[test]
    fn backspace_removes_large_paste_summary_in_one_keypress() {
        let mut composer = "prefix [sss +93330 words pasted]".to_string();
        let mut cursor = composer.len();
        let mut paste_buffer = Some("prefix this is a very large pasted message".to_string());

        let consumed = consume_large_paste_summary_with_backspace(
            &mut composer,
            &mut cursor,
            &mut paste_buffer,
        );

        assert!(consumed);
        assert_eq!(composer, "prefix ");
        assert_eq!(cursor, "prefix ".len());
        assert!(paste_buffer.is_none());
    }
}
