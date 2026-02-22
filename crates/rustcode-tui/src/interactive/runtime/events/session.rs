use rustcode_core::MessageRole;

pub(super) fn auto_rename_session(
    backend: &dyn crate::SessionBackend,
    session_id: &str,
    session_model: &str,
    _live_reasoning: &str,
    messages: Option<&[rustcode_core::StoredMessage]>,
) -> Option<rustcode_core::SessionInfo> {
    let messages = messages?;
    let first_user = messages.iter().find(|m| m.role == MessageRole::User)?;
    let text = first_user.content.as_str()?;
    let clean = text.trim().replace('\n', " ");
    if clean.is_empty() {
        return None;
    }
    let title = if clean.chars().count() > 50 {
        let short: String = clean.chars().take(50).collect();
        if let Some(pos) = short.rfind(' ') {
            format!("{}…", &short[..pos])
        } else {
            format!("{short}…")
        }
    } else {
        clean
    };
    let mut info = backend.update_session_title(session_id, Some(title)).ok()?;
    info.model = session_model.to_string();
    Some(info)
}
