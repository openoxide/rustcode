// ── rustcode-lsp ─────────────────────────────────────────────────────────
//
// Language Server Protocol client crate.
//
// Entry point: `LspManager::detect(workspace_root)` returns `Some(manager)`
// when a supported language server is found for the project, `None` otherwise.
//
// The manager lazily spawns the language server on first use and keeps it
// alive across agent turns. Operations are serialised through a `Mutex` since
// the agent processes tool calls sequentially.
//
// Supported servers (auto-detected, must be installed by user):
// - rust-analyzer (Rust / Cargo.toml)
// - gopls (Go / go.mod)
// - pyright-langserver (Python / pyproject.toml | setup.py | requirements.txt)
// - typescript-language-server (TypeScript / tsconfig.json | package.json)

mod client;
mod detect;
mod types;

pub use detect::ServerSpec;
pub use types::{Diagnostic, DiagnosticSeverity, FileDiagnostics, Location, WorkspaceSymbol};

use std::path::{Path, PathBuf};

use tokio::sync::Mutex;
use tracing::info;

// Re-export the client for use in the engine handlers
pub use client::LspClient;

/// Error type for LSP operations.
#[derive(Debug, thiserror::Error)]
pub enum LspError {
    /// Failed to spawn the language server process.
    #[error("failed to spawn language server: {0}")]
    Spawn(String),
    /// I/O error communicating with the language server.
    #[error("LSP I/O error: {0}")]
    Io(String),
    /// Request or initialisation timed out.
    #[error("LSP request timed out")]
    Timeout,
    /// `initialize` handshake failed.
    #[error("LSP initialize failed: {0}")]
    Initialize(String),
    /// No language server detected for this workspace.
    #[error("no language server detected for this workspace")]
    NoServer,
}

/// Manages a lazily-started language server for a workspace.
///
/// Construct with [`LspManager::detect`]; use the operation methods directly.
/// The underlying [`LspClient`] is created on first use and reused thereafter.
pub struct LspManager {
    /// Lazily initialised client; `None` before first use.
    client: Mutex<Option<LspClient>>,
    /// Detected server configuration.
    spec: ServerSpec,
    /// Workspace root for URI construction and server initialisation.
    workspace_root: PathBuf,
}

impl LspManager {
    /// Attempt to detect a language server for the given workspace root.
    ///
    /// Returns `None` if no supported project type is detected or the
    /// language server binary is not installed.
    #[must_use]
    pub fn detect(workspace_root: &Path) -> Option<Self> {
        let spec = detect::detect_server(workspace_root)?;
        info!(
            "LSP: detected '{}' for workspace {}",
            spec.server_id,
            workspace_root.display()
        );
        Some(Self {
            client: Mutex::new(None),
            spec,
            workspace_root: workspace_root.to_path_buf(),
        })
    }

    /// Get the server ID (e.g. `"rust-analyzer"`).
    #[must_use]
    pub fn server_id(&self) -> &str {
        self.spec.server_id
    }

    /// Shut down the language server if it was started.
    pub async fn shutdown(&self) {
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.take() {
            client.shutdown().await;
        }
    }

    // ── Internal ─────────────────────────────────────────────────────────

    /// Return a mutable guard to the client, starting it if needed.
    async fn get_or_start(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<LspClient>>, LspError> {
        let mut guard = self.client.lock().await;
        if guard.is_none() {
            let client =
                LspClient::start(&self.spec.command, &self.spec.args, &self.workspace_root).await?;
            *guard = Some(client);
        }
        Ok(guard)
    }

    // ── LSP operations (delegate to LspClient) ────────────────────────────

    /// Collect all diagnostics accumulated from `publishDiagnostics` notifications.
    ///
    /// Call after opening files (e.g. via `hover` or `definition`) so the
    /// server has had a chance to send diagnostics.
    pub async fn diagnostics(&self) -> Result<Vec<FileDiagnostics>, LspError> {
        let guard = self.get_or_start().await?;
        guard
            .as_ref()
            .ok_or_else(|| LspError::Io("client not initialised".to_string()))?
            .diagnostics()
            .await
    }

    /// Get hover documentation at a workspace-relative file position.
    pub async fn hover(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<String>, LspError> {
        let path = self.resolve(path);
        let guard = self.get_or_start().await?;
        guard
            .as_ref()
            .ok_or_else(|| LspError::Io("client not initialised".to_string()))?
            .hover(&path, line, character)
            .await
    }

    /// Get definition location(s) at a workspace-relative file position.
    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>, LspError> {
        let path = self.resolve(path);
        let guard = self.get_or_start().await?;
        guard
            .as_ref()
            .ok_or_else(|| LspError::Io("client not initialised".to_string()))?
            .definition(&path, line, character)
            .await
    }

    /// Search workspace symbols matching the given query string.
    pub async fn workspace_symbols(&self, query: &str) -> Result<Vec<WorkspaceSymbol>, LspError> {
        let guard = self.get_or_start().await?;
        guard
            .as_ref()
            .ok_or_else(|| LspError::Io("client not initialised".to_string()))?
            .workspace_symbols(query)
            .await
    }

    /// Resolve a relative path against the workspace root.
    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }
}
