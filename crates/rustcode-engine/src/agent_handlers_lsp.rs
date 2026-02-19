// ── LSP agent tool handler ────────────────────────────────────────────────
//
// Dispatches `lsp` tool calls to the workspace's language server via
// `Engine::lsp_manager` (a lazily-started `rustcode_lsp::LspManager`).
//
// Supported operations:
//   diagnostics         — return accumulated diagnostics (all files)
//   hover               — hover info at a position
//   definition          — go-to-definition location(s)
//   workspace_symbols   — symbol search by query string

use std::path::Path;

use rustcode_lsp::{LspError, LspManager};

use super::{CommandContext, Engine, ExecutionError};

impl Engine {
    /// Handle the `lsp` agent tool.
    ///
    /// Dispatches to the appropriate `LspManager` method based on `operation`.
    pub(crate) async fn agent_tool_lsp(
        &self,
        operation: &str,
        path: Option<&str>,
        line: Option<u32>,
        character: Option<u32>,
        query: Option<&str>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let manager = self.lsp_manager.as_deref().ok_or_else(|| {
            ExecutionError::Executor(
                "no language server detected for this workspace \
                 (install rust-analyzer, gopls, pyright, or typescript-language-server)"
                    .to_string(),
            )
        })?;

        match operation {
            "diagnostics" => lsp_diagnostics(manager).await,
            "hover" => {
                let path = require_path(path)?;
                let line = require_u32(line, "line")?;
                let character = require_u32(character, "character")?;
                lsp_hover(manager, path, line, character, &context.config.workspace_root).await
            }
            "definition" => {
                let path = require_path(path)?;
                let line = require_u32(line, "line")?;
                let character = require_u32(character, "character")?;
                lsp_definition(manager, path, line, character, &context.config.workspace_root).await
            }
            "workspace_symbols" => {
                let q = query.ok_or_else(|| {
                    ExecutionError::Dispatch(
                        "lsp workspace_symbols requires query".to_string(),
                    )
                })?;
                lsp_workspace_symbols(manager, q).await
            }
            other => Err(ExecutionError::Dispatch(format!(
                "unknown lsp operation '{other}'; expected: diagnostics, hover, definition, workspace_symbols"
            ))),
        }
    }
}

// ── Operation implementations ─────────────────────────────────────────────

async fn lsp_diagnostics(manager: &LspManager) -> Result<String, ExecutionError> {
    let file_diags = manager.diagnostics().await.map_err(lsp_err)?;

    if file_diags.is_empty() {
        return Ok("no diagnostics".to_string());
    }

    let mut out = String::new();
    let mut total = 0usize;
    for fd in &file_diags {
        for d in &fd.diagnostics {
            out.push_str(&d.display());
            out.push('\n');
            total += 1;
        }
    }
    out.push_str(&format!("\n{total} diagnostic(s)"));
    Ok(out)
}

async fn lsp_hover(
    manager: &LspManager,
    path: &str,
    line: u32,
    character: u32,
    workspace_root: &Path,
) -> Result<String, ExecutionError> {
    let resolved = resolve_path(path, workspace_root);
    match manager
        .hover(&resolved, line, character)
        .await
        .map_err(lsp_err)?
    {
        Some(text) => Ok(text),
        None => Ok("(no hover information at this position)".to_string()),
    }
}

async fn lsp_definition(
    manager: &LspManager,
    path: &str,
    line: u32,
    character: u32,
    workspace_root: &Path,
) -> Result<String, ExecutionError> {
    let resolved = resolve_path(path, workspace_root);
    let locations = manager
        .definition(&resolved, line, character)
        .await
        .map_err(lsp_err)?;

    if locations.is_empty() {
        return Ok("(no definition found at this position)".to_string());
    }

    let mut out = String::new();
    for loc in &locations {
        out.push_str(&loc.display());
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

async fn lsp_workspace_symbols(
    manager: &LspManager,
    query: &str,
) -> Result<String, ExecutionError> {
    let symbols = manager.workspace_symbols(query).await.map_err(lsp_err)?;

    if symbols.is_empty() {
        return Ok(format!("no symbols matching '{query}'"));
    }

    let mut out = String::new();
    for sym in &symbols {
        out.push_str(&format!(
            "{} [{}] — {}\n",
            sym.name,
            sym.kind,
            sym.location.display()
        ));
    }
    Ok(out.trim_end().to_string())
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn require_path(path: Option<&str>) -> Result<&str, ExecutionError> {
    path.ok_or_else(|| ExecutionError::Dispatch("lsp operation requires path".to_string()))
}

fn require_u32(val: Option<u32>, name: &str) -> Result<u32, ExecutionError> {
    val.ok_or_else(|| ExecutionError::Dispatch(format!("lsp operation requires {name}")))
}

fn resolve_path(path: &str, workspace_root: &Path) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    }
}

fn lsp_err(e: LspError) -> ExecutionError {
    ExecutionError::Executor(e.to_string())
}
