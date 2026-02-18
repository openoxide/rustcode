use super::*;

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
        let haystack = format!("{}\t{}", session.id, title).to_ascii_lowercase();
        if haystack.contains(&needle) {
            out.push(idx);
        }
    }
    out
}

pub(super) fn transcript_area_height(state: &AppState) -> u16 {
    state.last_area.height.saturating_sub(7).max(1)
}

pub(super) fn sort_sessions(sessions: &mut Vec<SessionInfo>) {
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
        return format!("{}s", secs);
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{}m", mins);
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{}h", hours);
    }
    let days = hours / 24;
    format!("{}d", days)
}
