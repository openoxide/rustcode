// ── rustcode-lsp output types ────────────────────────────────────────────
//
// These are rustcode-specific output types returned from LSP operations.
// They are distinct from the wire types (which are constructed as serde_json::Value).

/// Severity level of a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// Compile error or type error.
    Error,
    /// Warning from linter or compiler.
    Warning,
    /// Informational hint.
    Information,
    /// Stylistic hint.
    Hint,
}

impl DiagnosticSeverity {
    /// Convert from LSP integer severity (1=Error, 2=Warning, 3=Info, 4=Hint).
    #[must_use]
    pub fn from_u32(n: u32) -> Self {
        match n {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            _ => Self::Hint,
        }
    }

    /// Short label for display.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }
}

/// A single diagnostic from the language server.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Workspace-relative file path (or URI if outside workspace).
    pub file: String,
    /// 0-indexed start line.
    pub start_line: u32,
    /// 0-indexed start character.
    pub start_char: u32,
    /// 0-indexed end line.
    pub end_line: u32,
    /// 0-indexed end character.
    pub end_char: u32,
    /// Severity classification.
    pub severity: DiagnosticSeverity,
    /// Human-readable message.
    pub message: String,
}

impl Diagnostic {
    /// Format as `file:line:char: severity: message`.
    #[must_use]
    pub fn display(&self) -> String {
        format!(
            "{}:{}:{}: {}: {}",
            self.file,
            self.start_line + 1,
            self.start_char + 1,
            self.severity.label(),
            self.message
        )
    }
}

/// All diagnostics for one file.
#[derive(Debug, Clone)]
pub struct FileDiagnostics {
    /// Raw LSP URI (e.g. `file:///path/to/file.rs`).
    pub uri: String,
    /// Diagnostics for this file.
    pub diagnostics: Vec<Diagnostic>,
}

/// A source location (file + position).
#[derive(Debug, Clone)]
pub struct Location {
    /// Raw LSP URI.
    pub uri: String,
    /// 0-indexed line number.
    pub line: u32,
    /// 0-indexed character offset.
    pub character: u32,
}

impl Location {
    /// Format as `uri:line+1:char+1`.
    #[must_use]
    pub fn display(&self) -> String {
        format!("{}:{}:{}", self.uri, self.line + 1, self.character + 1)
    }
}

/// A workspace symbol match.
#[derive(Debug, Clone)]
pub struct WorkspaceSymbol {
    /// Symbol name.
    pub name: String,
    /// Symbol kind string (Function, Struct, etc.).
    pub kind: String,
    /// Declaration location.
    pub location: Location,
}
