use std::path::PathBuf;

pub(crate) fn default_sessions_root() -> PathBuf {
    if let Ok(path) = std::env::var("RUSTCODE_SESSIONS_DIR") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("rustcode/sessions");
    }
    if let Ok(path) = std::env::var("HOME") {
        return PathBuf::from(path).join(".local/share/rustcode/sessions");
    }
    PathBuf::from(".rustcode-sessions")
}

pub(crate) fn default_prompt_history_path() -> PathBuf {
    if let Ok(path) = std::env::var("RUSTCODE_PROMPT_HISTORY_FILE") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("rustcode/prompt-history.json");
    }
    if let Ok(path) = std::env::var("HOME") {
        return PathBuf::from(path).join(".local/share/rustcode/prompt-history.json");
    }
    PathBuf::from(".rustcode-prompt-history.json")
}
