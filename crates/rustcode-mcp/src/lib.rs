use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-11-25";

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const STDIO_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

mod stdio;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("invalid mcp endpoint url: {0}")]
    InvalidUrl(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("process error: {0}")]
    Process(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpResource {
    pub uri: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "mimeType")]
    pub mime_type: Option<String>,
}

#[derive(Debug)]
pub struct McpHttpSession {
    endpoint: reqwest::Url,
    http: reqwest::Client,
    bearer_token: Option<String>,
    protocol_version: String,
    session_id: Option<String>,
    next_id: AtomicU64,
}

#[derive(Debug)]
pub struct McpStdioSession {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    protocol_version: String,
    next_id: AtomicU64,
}

impl McpHttpSession {
    pub async fn connect(endpoint: &str, bearer_token: Option<String>) -> Result<Self, McpError> {
        let endpoint =
            reqwest::Url::parse(endpoint).map_err(|err| McpError::InvalidUrl(err.to_string()))?;
        if endpoint.scheme() != "https" && endpoint.scheme() != "http" {
            return Err(McpError::InvalidUrl(
                "endpoint must use http or https".to_string(),
            ));
        }

        let http = reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|err| McpError::Http(format!("failed to build http client: {err}")))?;

        let mut session = Self {
            endpoint,
            http,
            bearer_token,
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_string(),
            session_id: None,
            next_id: AtomicU64::new(1),
        };

        session.initialize().await?;
        Ok(session)
    }

    #[must_use]
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..32 {
            let params = match cursor.as_deref() {
                None => Value::Object(serde_json::Map::new()),
                Some(cursor) => serde_json::json!({"cursor": cursor}),
            };
            let result = self.request("tools/list", params).await?;
            let tools = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpError::Protocol("tools/list result missing tools array".to_string())
                })?;
            for item in tools {
                let tool: McpTool = serde_json::from_value(item.clone())
                    .map_err(|err| McpError::Protocol(format!("invalid tool entry: {err}")))?;
                all.push(tool);
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok(all)
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({"name": name, "arguments": arguments}),
            )
            .await?;
        Ok(result)
    }

    pub async fn list_resources(&self) -> Result<Vec<McpResource>, McpError> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..32 {
            let params = match cursor.as_deref() {
                None => Value::Object(serde_json::Map::new()),
                Some(cursor) => serde_json::json!({"cursor": cursor}),
            };
            let result = self.request("resources/list", params).await?;
            let resources = result
                .get("resources")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpError::Protocol("resources/list result missing resources array".to_string())
                })?;
            for item in resources {
                let resource: McpResource = serde_json::from_value(item.clone())
                    .map_err(|err| McpError::Protocol(format!("invalid resource entry: {err}")))?;
                all.push(resource);
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            if cursor.is_none() {
                break;
            }
        }
        Ok(all)
    }

    pub async fn read_resource(&self, uri: &str) -> Result<Value, McpError> {
        self.request("resources/read", serde_json::json!({"uri": uri}))
            .await
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        let init_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "rustcode",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        });

        let (response, headers) = self.post_jsonrpc(&init_request, false).await?;
        let negotiated = response
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION);
        self.protocol_version = negotiated.to_string();

        if let Some(session_id) = headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            if !session_id.trim().is_empty() {
                self.session_id = Some(session_id.to_string());
            }
        }

        // Per spec, after initialization the client must send notifications/initialized.
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        self.post_notification(&initialized).await?;

        Ok(())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (response, _headers) = self.post_jsonrpc(&request, true).await?;
        match response.get("error") {
            Some(err) => Err(McpError::Protocol(format!(
                "jsonrpc error: {}",
                err.to_string()
            ))),
            None => response
                .get("result")
                .cloned()
                .ok_or_else(|| McpError::Protocol("jsonrpc response missing result".to_string())),
        }
    }

    async fn post_notification(&self, message: &Value) -> Result<(), McpError> {
        let mut headers = self.base_headers(false)?;
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(self.endpoint.clone())
            .headers(headers)
            .json(message)
            .send()
            .await
            .map_err(|err| McpError::Http(err.to_string()))?;

        let status = response.status();
        if status.as_u16() == 202 {
            return Ok(());
        }
        if status.is_success() {
            return Ok(());
        }
        Err(McpError::Http(format!(
            "notification rejected with status={}",
            status
        )))
    }

    async fn post_jsonrpc(
        &self,
        message: &Value,
        include_protocol_headers: bool,
    ) -> Result<(Value, HeaderMap), McpError> {
        let mut headers = self.base_headers(include_protocol_headers)?;
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(self.endpoint.clone())
            .headers(headers)
            .json(message)
            .send()
            .await
            .map_err(|err| McpError::Http(err.to_string()))?;

        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            return Err(McpError::Http(format!("request failed: {status}")));
        }

        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let body = response
            .text()
            .await
            .map_err(|err| McpError::Http(err.to_string()))?;

        let response_json = if content_type.contains("text/event-stream") {
            extract_first_jsonrpc_from_sse(&body).ok_or_else(|| {
                McpError::Protocol("sse response did not include jsonrpc payload".to_string())
            })?
        } else {
            serde_json::from_str::<Value>(&body)
                .map_err(|err| McpError::Protocol(format!("invalid json response: {err}")))?
        };

        Ok((response_json, headers))
    }

    fn base_headers(&self, include_protocol_headers: bool) -> Result<HeaderMap, McpError> {
        let mut headers = HeaderMap::new();
        if let Some(token) = self.bearer_token.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|err| McpError::Protocol(err.to_string()))?;
            headers.insert(AUTHORIZATION, value);
        }
        if let Some(session_id) = self.session_id.as_deref() {
            let value = HeaderValue::from_str(session_id)
                .map_err(|err| McpError::Protocol(err.to_string()))?;
            headers.insert("mcp-session-id", value);
        }
        if include_protocol_headers {
            let value = HeaderValue::from_str(&self.protocol_version)
                .map_err(|err| McpError::Protocol(err.to_string()))?;
            headers.insert("mcp-protocol-version", value);
        }
        Ok(headers)
    }
}

fn is_matching_id(id_value: Option<&Value>, id: u64) -> bool {
    match id_value {
        Some(Value::Number(value)) => value.as_u64() == Some(id),
        Some(Value::String(value)) => value.parse::<u64>().ok() == Some(id),
        _ => false,
    }
}

async fn read_jsonrpc_frame<R>(reader: &mut R) -> Result<Value, McpError>
where
    R: AsyncBufRead + AsyncRead + Unpin,
{
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|err| McpError::Io(err.to_string()))?;
        if n == 0 {
            return Err(McpError::Io(
                "unexpected EOF while reading frame headers".to_string(),
            ));
        }
        if line == "\r\n" {
            break;
        }
        let trimmed = line.trim_end();
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                let parsed = value.trim().parse::<usize>().map_err(|err| {
                    McpError::Protocol(format!("invalid Content-Length header: {err}"))
                })?;
                content_length = Some(parsed);
            }
        }
    }

    let length = content_length.ok_or_else(|| {
        McpError::Protocol("missing Content-Length header in stdio response".to_string())
    })?;
    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|err| McpError::Io(err.to_string()))?;
    serde_json::from_slice::<Value>(&payload)
        .map_err(|err| McpError::Protocol(format!("invalid stdio json response: {err}")))
}

fn extract_first_jsonrpc_from_sse(body: &str) -> Option<Value> {
    // Minimal SSE parser: collect `data:` lines per event and parse JSON if non-empty.
    // Good enough for MCP server responses that send JSON-RPC payloads in SSE `data`.
    for event in body.split("\n\n") {
        let mut data_lines = Vec::new();
        for line in event.lines() {
            let trimmed = line.trim_end();
            if let Some(rest) = trimmed.strip_prefix("data:") {
                data_lines.push(rest.trim_start());
            }
        }
        let data = data_lines.join("\n");
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            if value.get("jsonrpc").and_then(Value::as_str) == Some("2.0") {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
