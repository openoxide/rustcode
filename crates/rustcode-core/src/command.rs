#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOptions {
    pub max_steps: usize,
    pub max_tool_calls_per_step: usize,
    pub allow_write: bool,
    pub allow_edit: bool,
    pub allow_exec: bool,
    pub max_read_bytes: usize,
    pub max_list_entries: usize,
    pub max_tool_result_bytes: usize,
    pub max_write_bytes: usize,
    /// Optional mode hint injected into the system prompt (e.g. "build", "plan").
    pub mode_hint: Option<String>,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            max_steps: 300,
            max_tool_calls_per_step: 300,
            allow_write: false,
            allow_edit: false,
            allow_exec: false,
            // Align with OpenCode truncation defaults (50 KiB).
            max_read_bytes: 50 * 1024,
            max_list_entries: 2000,
            max_tool_result_bytes: 50 * 1024,
            max_write_bytes: 256 * 1024,
            mode_hint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run {
        prompt: String,
    },
    Agent {
        prompt: String,
        options: AgentOptions,
        history: Vec<StoredMessage>,
    },
    Exec {
        command: String,
        args: Vec<String>,
    },
    List {
        path: Option<String>,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
        contents: String,
    },
    Edit {
        path: String,
        from: String,
        to: String,
    },
    Tui,
    Serve {
        listen: String,
    },
    Version,
}
use crate::session::StoredMessage;
