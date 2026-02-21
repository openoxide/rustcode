mod execute;
mod session_ops;
mod slash;

pub(super) use execute::execute_command;
pub(super) use session_ops::{open_session_by_id, refresh_chat_messages};
pub(super) use slash::handle_slash_command;

/// All slash commands available in the composer, with short descriptions.
///
/// Shown in the [`Modal::SlashHelp`] autocomplete popup.
pub(super) const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/clear", "Clear composer input"),
    ("/delete", "Delete current session"),
    ("/find <query>", "Search transcript"),
    ("/fork", "Fork current session  (Ctrl+F)"),
    ("/help", "Show keybindings and tips"),
    ("/memory", "View memory summary (full modal)"),
    ("/memory on", "Enable memory collection"),
    ("/memory off", "Disable memory collection"),
    ("/memory clear", "Clear all memories (with confirmation)"),
    ("/model", "Switch LLM model  (Ctrl+M)"),
    ("/new", "Create a new session  (Ctrl+N)"),
    ("/providers", "Connect/disconnect providers  (Ctrl+A)"),
    ("/refresh", "Reload transcript  (Ctrl+R)"),
    ("/rename <title>", "Rename current session"),
    (
        "/resume <id>",
        "Resume a session by ID (empty = session list)",
    ),
    ("/sessions", "Go to sessions screen  (Ctrl+Q)"),
    ("/skill", "List or inject a skill  (Ctrl+S)"),
];

/// Filter `entries` (each a `"provider/model"` string) by case-insensitive substring match.
pub(super) fn filter_models(entries: &[String], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    let needle = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.to_ascii_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}
