use super::{
    approval_options_count, build_transcript_lines, AppState, ChatState, Duration, Screen,
    SessionInfo, SystemTime, Toast, ToastVariant,
};

pub(super) fn push_toast(
    state: &mut AppState,
    variant: ToastVariant,
    message: impl Into<String>,
    ttl: Duration,
) {
    let message = message.into();
    if message.trim().is_empty() {
        return;
    }
    let toast = Toast {
        variant,
        message,
        expires_at: SystemTime::now() + ttl,
    };
    state.toasts.push(toast);
    while state.toasts.len() > 6 {
        state.toasts.remove(0);
    }
}

pub(super) fn drain_toasts(state: &mut AppState) {
    let now = SystemTime::now();
    state.toasts.retain(|toast| toast.expires_at > now);
}

pub(super) fn compute_sessions_view(sessions: &[SessionInfo], filter: &str) -> Vec<usize> {
    if filter.trim().is_empty() {
        return (0..sessions.len()).collect();
    }
    let needle = filter.to_ascii_lowercase();
    let mut out = Vec::new();
    for (idx, session) in sessions.iter().enumerate() {
        let title = session.title.as_deref().unwrap_or("");
        let branch = session.branch.trim();
        let haystack = format!("{}\t{}\t{}", session.id, title, branch).to_ascii_lowercase();
        if haystack.contains(&needle) {
            out.push(idx);
        }
    }
    out
}

pub(super) fn transcript_area_height(state: &AppState) -> u16 {
    state.last_area.height.saturating_sub(7).max(1)
}

/// Compute how tall the inline viewport should be for the current state.
///
/// Returns a height in rows that grows with content but is capped at the
/// terminal height stored in `state.last_area`.
#[must_use]
pub(super) fn compute_desired_height(state: &AppState) -> u16 {
    let term_height = state.last_area.height;
    if term_height == 0 {
        return 8; // safe minimum before first size query
    }
    match &state.screen {
        Screen::Sessions => sessions_desired_height(state, term_height),
        Screen::Chat(chat) => chat_desired_height(state, chat, term_height),
    }
}

/// Desired height for the sessions list screen.
fn sessions_desired_height(state: &AppState, term_height: u16) -> u16 {
    // List items + 2 borders
    let list_h = (state.sessions_view.len() as u16 + 2).max(5);
    // Search bar: 3 rows when active, 0 otherwise
    let search_h = if state.sessions_filter_active {
        3u16
    } else {
        0
    };
    // Footer: 3 rows (border + 2 lines)
    let footer_h = 3u16;
    (list_h + search_h + footer_h).min(term_height)
}

/// Desired height for the chat screen.
///
/// Sum of transcript + composer + status bar, growing with content.
fn chat_desired_height(state: &AppState, chat: &ChatState, term_height: u16) -> u16 {
    let width = state.last_area.width.max(1);

    // Composer height: count visual wrapped lines (not just \n lines)
    // so the viewport grows when text wraps within the composer.
    let composer_inner_w = if chat.activity_hidden {
        width.saturating_sub(2).max(1) as usize
    } else {
        (width * 72 / 100).saturating_sub(2).max(1) as usize
    };
    let composer_visual_lines = composer_wrapped_line_count(&chat.composer, composer_inner_w);
    let composer_h = (composer_visual_lines as u16 + 2).max(5);
    // Cap composer at half terminal height.
    let composer_h = composer_h.min(term_height / 2);

    // Override with approval selector height when pending.
    let bottom_h = if let Some(pending) = &state.pending_approval {
        let opt_count = approval_options_count(&pending.request) as u16;
        opt_count + 3
    } else {
        composer_h
    };

    // Status bar: always 3 rows.
    let status_h = 3u16;

    // Transcript: count visual wrapped lines.
    let transcript_lines = build_transcript_lines(chat);
    let inner_w = width.saturating_sub(2).max(1) as usize;
    let wrapped: usize = transcript_lines
        .iter()
        .map(|l| {
            let w = l.width();
            if w <= inner_w {
                1
            } else {
                w.div_ceil(inner_w)
            }
        })
        .sum();
    // Transcript box = wrapped lines + 2 borders.
    // Minimum height: ~40% of terminal height so the transcript is always
    // prominent relative to the composer, with a floor of 8 rows.
    let forty_pct = (term_height * 2 / 5).max(8);
    let transcript_min = forty_pct.max(bottom_h);
    let transcript_h = (wrapped as u16 + 2).max(transcript_min);

    // Mode bar: 1 row between transcript and composer.
    let mode_bar_h = 1u16;

    (transcript_h + mode_bar_h + bottom_h + status_h).min(term_height)
}

/// Count the number of visual lines `text` occupies when word-wrapped at
/// `width` columns.  Matches the wrapping logic in
/// `render_main::chat::char_wrap_text` and `composer_cursor_visual`.
fn composer_wrapped_line_count(text: &str, width: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    if text.is_empty() {
        return 1;
    }
    let width = width.max(1);
    let mut lines = 1usize;
    let mut col = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\n' {
            lines += 1;
            col = 0;
            i += 1;
            continue;
        }
        if ch == ' ' || ch == '\t' {
            let cw = ch.width().unwrap_or(1);
            col += cw;
            if col >= width {
                lines += 1;
                col = 0;
            }
            i += 1;
            continue;
        }
        // Word — measure full width.
        let mut word_w = 0usize;
        let mut j = i;
        while j < chars.len() && chars[j] != ' ' && chars[j] != '\t' && chars[j] != '\n' {
            word_w += chars[j].width().unwrap_or(1);
            j += 1;
        }
        if word_w <= width && col > 0 && col + word_w > width {
            lines += 1;
            col = 0;
        }
        for ch in chars.iter().take(j).skip(i) {
            let cw = ch.width().unwrap_or(1);
            if col > 0 && col + cw > width {
                lines += 1;
                col = 0;
            }
            col += cw;
        }
        i = j;
    }
    lines
}

pub(super) fn sort_sessions(sessions: &mut [SessionInfo]) {
    sessions.sort_by(|a, b| b.updated_at_unix_ms.cmp(&a.updated_at_unix_ms));
}

pub(super) fn format_age(updated_at_unix_ms: i64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64;
    let delta_ms = (now_ms - updated_at_unix_ms).max(0);
    let secs = delta_ms / 1000;
    if secs < 10 {
        return "now".to_string();
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    format!("{days}d")
}
