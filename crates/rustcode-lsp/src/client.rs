// ── LSP JSON-RPC client ───────────────────────────────────────────────────
//
// Custom JSON-RPC transport over stdin/stdout using Content-Length framing.
// No external LSP framework — the protocol is simple enough to implement
// directly and keeps our dependency surface small.
//
// Reference: LSP spec §3.1 (Base Protocol)

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, warn};

use crate::types::{Diagnostic, DiagnosticSeverity, FileDiagnostics, Location, WorkspaceSymbol};
use crate::LspError;

#[cfg(test)]
mod tests;

// ── Timeouts ──────────────────────────────────────────────────────────────

/// Timeout for the `initialize` handshake (language servers can be slow to start).
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for individual LSP requests.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Channel capacity for the writer task.
const CHANNEL_CAPACITY: usize = 64;

// ── Internal representation of a published diagnostic ─────────────────────

/// Raw diagnostic as received from `textDocument/publishDiagnostics`.
#[derive(Debug, Clone)]
pub(crate) struct PublishedDiagnostic {
    pub range_start_line: u32,
    pub range_start_char: u32,
    pub range_end_line: u32,
    pub range_end_char: u32,
    pub severity: u32,
    pub message: String,
}

// ── LspClient ─────────────────────────────────────────────────────────────

/// Active LSP client connected to a language server subprocess.
///
/// Manages two background tasks:
/// - A **writer task** that frames and writes JSON-RPC messages to stdin.
/// - A **reader task** that reads and routes JSON-RPC messages from stdout.
///
/// The language server subprocess is kept alive as long as this struct exists.
/// Dropping `LspClient` kills the process (via `kill_on_drop(true)`).
pub struct LspClient {
    sender: mpsc::Sender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    diagnostics: Arc<Mutex<HashMap<String, Vec<PublishedDiagnostic>>>>,
    next_id: AtomicU64,
    workspace_root: std::path::PathBuf,
    // Kept alive for process lifetime; killed on drop.
    _child: Mutex<Child>,
}

impl LspClient {
    /// Spawn `cmd args` and perform the `initialize`/`initialized` handshake.
    ///
    /// Returns an initialised client ready for use. Times out after 30 s.
    pub async fn start(
        cmd: &str,
        args: &[String],
        workspace_root: &Path,
    ) -> Result<Self, LspError> {
        use std::process::Stdio;
        use tokio::process::Command;

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // suppress server log noise
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| LspError::Spawn(format!("failed to spawn '{cmd}': {e}")))?;

        let stdin: ChildStdin = child
            .stdin
            .take()
            .ok_or_else(|| LspError::Io("child has no stdin".to_string()))?;
        let stdout: ChildStdout = child
            .stdout
            .take()
            .ok_or_else(|| LspError::Io("child has no stdout".to_string()))?;

        let (tx, rx) = mpsc::channel::<String>(CHANNEL_CAPACITY);
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::default();
        let diagnostics: Arc<Mutex<HashMap<String, Vec<PublishedDiagnostic>>>> = Arc::default();

        // Launch background I/O tasks
        tokio::spawn(writer_task(stdin, rx));
        tokio::spawn(reader_task(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&diagnostics),
        ));

        let client = LspClient {
            sender: tx,
            pending,
            diagnostics,
            next_id: AtomicU64::new(1),
            workspace_root: workspace_root.to_path_buf(),
            _child: Mutex::new(child),
        };

        // Perform LSP handshake
        client.initialize(workspace_root).await?;

        Ok(client)
    }

    // ── LSP handshake ─────────────────────────────────────────────────────

    async fn initialize(&self, workspace_root: &Path) -> Result<(), LspError> {
        let root_uri = path_to_uri(workspace_root)?;
        #[allow(clippy::cast_possible_truncation)]
        let pid = std::process::id();

        let params = json!({
            "processId": pid,
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": {},
                    "publishDiagnostics": { "relatedInformation": false }
                },
                "workspace": {
                    "symbol": {}
                }
            }
        });

        tokio::time::timeout(INIT_TIMEOUT, self.request("initialize", params))
            .await
            .map_err(|_| LspError::Timeout)?
            .map_err(|e| LspError::Initialize(e.to_string()))?;

        // Notify that client is ready (no response expected)
        self.notify("initialized", json!({})).await?;

        Ok(())
    }

    // ── Core request/notify helpers ───────────────────────────────────────

    async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        self.sender
            .send(msg.to_string())
            .await
            .map_err(|_| LspError::Io("LSP server disconnected".to_string()))?;

        tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| LspError::Timeout)?
            .map_err(|_| LspError::Io("response channel closed".to_string()))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.sender
            .send(msg.to_string())
            .await
            .map_err(|_| LspError::Io("LSP server disconnected".to_string()))
    }

    // ── File helper ───────────────────────────────────────────────────────

    /// Send `textDocument/didOpen` so the server knows about the file.
    async fn did_open(&self, path: &Path) -> Result<String, LspError> {
        let uri = path_to_uri(path)?;
        let language_id = language_id_from_path(path);
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| LspError::Io(format!("failed to read '{}': {e}", path.display())))?;

        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": text,
                }
            }),
        )
        .await?;

        Ok(uri)
    }

    // ── Public LSP operations ─────────────────────────────────────────────

    /// Return all accumulated diagnostics (collected from `publishDiagnostics` notifications).
    ///
    /// Note: diagnostics are pushed by the server asynchronously; call this
    /// after a small delay or after triggering a file open/save.
    pub async fn diagnostics(&self) -> Result<Vec<FileDiagnostics>, LspError> {
        let map = self.diagnostics.lock().await;
        let mut result = Vec::new();
        for (uri, diags) in map.iter() {
            let diagnostics: Vec<Diagnostic> = diags
                .iter()
                .map(|d| Diagnostic {
                    file: uri_to_display(uri, &self.workspace_root),
                    start_line: d.range_start_line,
                    start_char: d.range_start_char,
                    end_line: d.range_end_line,
                    end_char: d.range_end_char,
                    severity: DiagnosticSeverity::from_u32(d.severity),
                    message: d.message.clone(),
                })
                .collect();
            if !diagnostics.is_empty() {
                result.push(FileDiagnostics {
                    uri: uri.clone(),
                    diagnostics,
                });
            }
        }
        Ok(result)
    }

    /// Get hover information at a position.
    pub async fn hover(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<String>, LspError> {
        let uri = self.did_open(path).await?;

        let response = self
            .request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character },
                }),
            )
            .await?;

        let result = &response["result"];
        if result.is_null() {
            return Ok(None);
        }

        let contents = &result["contents"];
        let text = if let Some(v) = contents.get("value").and_then(Value::as_str) {
            // MarkupContent { kind, value }
            v.to_string()
        } else if let Some(s) = contents.as_str() {
            // Plain string
            s.to_string()
        } else if let Some(arr) = contents.as_array() {
            // Array of MarkedString
            arr.iter()
                .filter_map(|item| {
                    item.get("value")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            return Ok(None);
        };

        Ok(if text.is_empty() { None } else { Some(text) })
    }

    /// Get definition location(s) at a position.
    pub async fn definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>, LspError> {
        let uri = self.did_open(path).await?;

        let response = self
            .request(
                "textDocument/definition",
                json!({
                    "textDocument": { "uri": uri },
                    "position": { "line": line, "character": character },
                }),
            )
            .await?;

        let result = &response["result"];
        if result.is_null() {
            return Ok(Vec::new());
        }

        let locations = match result {
            Value::Array(arr) => arr.iter().filter_map(parse_location).collect(),
            obj @ Value::Object(_) => parse_location(obj).into_iter().collect(),
            _ => Vec::new(),
        };

        Ok(locations)
    }

    /// Search workspace symbols matching a query string (returns first 20).
    pub async fn workspace_symbols(&self, query: &str) -> Result<Vec<WorkspaceSymbol>, LspError> {
        let response = self
            .request("workspace/symbol", json!({ "query": query }))
            .await?;

        let result = &response["result"];
        if result.is_null() {
            return Ok(Vec::new());
        }

        let symbols = result
            .as_array()
            .map(|arr| {
                arr.iter()
                    .take(20)
                    .filter_map(parse_workspace_symbol)
                    .collect()
            })
            .unwrap_or_default();

        Ok(symbols)
    }

    /// Send the `shutdown` request then the `exit` notification.
    pub async fn shutdown(&self) {
        let _ = self.request("shutdown", json!(null)).await;
        let _ = self.notify("exit", json!(null)).await;
    }
}

// ── Background tasks ──────────────────────────────────────────────────────

/// Frame and write JSON-RPC messages to the language server stdin.
async fn writer_task(mut stdin: ChildStdin, mut rx: mpsc::Receiver<String>) {
    while let Some(body) = rx.recv().await {
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        if stdin.write_all(frame.as_bytes()).await.is_err() {
            break; // stdin closed (process exited)
        }
    }
}

/// Read Content-Length framed messages from the language server stdout and
/// route them to the correct pending request channel or the diagnostics map.
async fn reader_task(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    diagnostics: Arc<Mutex<HashMap<String, Vec<PublishedDiagnostic>>>>,
) {
    let mut reader = BufReader::new(stdout);

    loop {
        // ── Parse headers ─────────────────────────────────────────────────
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => return, // EOF
                Err(_) => return,
                Ok(_) => {}
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break; // blank line terminates headers
            }
            if let Some(n) = trimmed.strip_prefix("Content-Length: ") {
                content_length = n.trim().parse().ok();
            }
        }

        let Some(len) = content_length else {
            warn!("LSP: message without Content-Length header, skipping");
            continue;
        };

        // ── Read body ─────────────────────────────────────────────────────
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).await.is_err() {
            return; // connection closed
        }

        let msg: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                warn!("LSP: JSON parse error: {e}");
                continue;
            }
        };
        debug!(
            "LSP recv: method={:?} id={:?}",
            msg.get("method"),
            msg.get("id")
        );

        // ── Route the message ─────────────────────────────────────────────
        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
            // Response to a previous request
            let tx = pending.lock().await.remove(&id);
            if let Some(tx) = tx {
                let _ = tx.send(msg);
            }
        } else if let Some(method) = msg.get("method").and_then(Value::as_str) {
            // Server-initiated notification
            if method == "textDocument/publishDiagnostics" {
                handle_publish_diagnostics(&msg, &diagnostics).await;
            }
            // Other notifications (window/logMessage, etc.) are intentionally ignored
        }
    }
}

/// Accumulate diagnostics from a `publishDiagnostics` notification.
async fn handle_publish_diagnostics(
    msg: &Value,
    diagnostics: &Mutex<HashMap<String, Vec<PublishedDiagnostic>>>,
) {
    let Some(uri) = msg["params"]["uri"].as_str() else {
        return;
    };
    let Some(diags) = msg["params"]["diagnostics"].as_array() else {
        return;
    };

    let parsed: Vec<PublishedDiagnostic> = diags
        .iter()
        .filter_map(|d| {
            let msg_text = d["message"].as_str()?;
            let severity = d["severity"].as_u64().unwrap_or(1) as u32;
            let start = &d["range"]["start"];
            let end = &d["range"]["end"];
            Some(PublishedDiagnostic {
                range_start_line: start["line"].as_u64()? as u32,
                range_start_char: start["character"].as_u64()? as u32,
                range_end_line: end["line"].as_u64()? as u32,
                range_end_char: end["character"].as_u64()? as u32,
                severity,
                message: msg_text.to_string(),
            })
        })
        .collect();

    diagnostics.lock().await.insert(uri.to_string(), parsed);
}

// ── URI utilities ─────────────────────────────────────────────────────────

/// Convert a filesystem path to an LSP `file://` URI.
pub(crate) fn path_to_uri(path: &Path) -> Result<String, LspError> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| LspError::Io(format!("current_dir error: {e}")))?
            .join(path)
    };
    let s = abs
        .to_str()
        .ok_or_else(|| LspError::Io("path is not valid UTF-8".to_string()))?;
    Ok(format!("file://{s}"))
}

/// Convert an LSP URI back to a display path (workspace-relative if possible).
pub(crate) fn uri_to_display(uri: &str, workspace_root: &Path) -> String {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    std::path::Path::new(path)
        .strip_prefix(workspace_root)
        .map_or_else(|_| path.to_string(), |p| p.display().to_string())
}

// ── Language ID helper ────────────────────────────────────────────────────

fn language_id_from_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("go") => "go",
        Some("py") => "python",
        Some("ts" | "tsx") => "typescript",
        Some("js" | "jsx") => "javascript",
        Some("c" | "h") => "c",
        Some("cpp" | "hpp" | "cc") => "cpp",
        Some("java") => "java",
        Some("rb") => "ruby",
        _ => "plaintext",
    }
}

// ── Response parsing helpers ──────────────────────────────────────────────

fn parse_location(v: &Value) -> Option<Location> {
    // Handles both `Location` and `LocationLink` shapes
    let uri = v.get("uri").or_else(|| v.get("targetUri"))?.as_str()?;
    let range = v.get("range").or_else(|| v.get("targetSelectionRange"))?;
    let line = range["start"]["line"].as_u64()? as u32;
    let character = range["start"]["character"].as_u64()? as u32;
    Some(Location {
        uri: uri.to_string(),
        line,
        character,
    })
}

fn parse_workspace_symbol(v: &Value) -> Option<WorkspaceSymbol> {
    let name = v["name"].as_str()?;
    let kind = symbol_kind_name(v["kind"].as_u64().unwrap_or(0));
    let location = parse_location(&v["location"])?;
    Some(WorkspaceSymbol {
        name: name.to_string(),
        kind: kind.to_string(),
        location,
    })
}

fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Unknown",
    }
}
