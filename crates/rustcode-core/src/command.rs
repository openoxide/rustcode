#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run { prompt: String },
    Exec { command: String, args: Vec<String> },
    List { path: Option<String> },
    Read { path: String },
    Write { path: String, contents: String },
    Tui,
    Serve { listen: String },
    Version,
}
