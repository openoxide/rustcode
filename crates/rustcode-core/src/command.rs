#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run { prompt: String },
    Exec { command: String, args: Vec<String> },
    Tui,
    Serve { listen: String },
    Version,
}
