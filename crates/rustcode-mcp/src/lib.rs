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

impl McpStdioSession {
    pub async fn connect(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        bearer_token: Option<String>,
    ) -> Result<Self, McpError> {
        if command.trim().is_empty() {
            return Err(McpError::InvalidUrl(
                "stdio command must not be empty".to_string(),
            ));
        }

        let mut child_command = Command::new(command);
        child_command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in env {
            child_command.env(key, value);
        }
        if let Some(token) = bearer_token {
            child_command.env("MCP_AUTH_TOKEN", token);
        }

        let mut child = child_command.spawn().map_err(|err| {
            McpError::Process(format!("failed to spawn mcp stdio command: {err}"))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Process("failed to capture child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Process("failed to capture child stdout".to_string()))?;

        let mut session = Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_string(),
            next_id: AtomicU64::new(1),
        };
        session.initialize().await?;
        Ok(session)
    }

    #[must_use]
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
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
        self.request(
            "tools/call",
            serde_json::json!({"name": name, "arguments": arguments}),
        )
        .await
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

        let response = self
            .send_request_expect_response(init_id, &init_request)
            .await?;
        let negotiated = response
            .get("result")
            .and_then(|result| result.get("protocolVersion"))
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL_VERSION);
        self.protocol_version = negotiated.to_string();

        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        self.send_notification(&initialized).await?;

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

        let response = self.send_request_expect_response(id, &request).await?;
        match response.get("error") {
            Some(err) => Err(McpError::Protocol(format!("jsonrpc error: {}", err))),
            None => response
                .get("result")
                .cloned()
                .ok_or_else(|| McpError::Protocol("jsonrpc response missing result".to_string())),
        }
    }

    async fn send_notification(&self, message: &Value) -> Result<(), McpError> {
        self.write_message(message).await
    }

    async fn send_request_expect_response(
        &self,
        id: u64,
        message: &Value,
    ) -> Result<Value, McpError> {
        self.write_message(message).await?;
        for _ in 0..64 {
            let response = tokio::time::timeout(STDIO_RESPONSE_TIMEOUT, self.read_message())
                .await
                .map_err(|_| {
                    McpError::Io(format!("timed out waiting for stdio mcp response id={id}"))
                })??;
            if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                continue;
            }
            if is_matching_id(response.get("id"), id) {
                return Ok(response);
            }
        }
        Err(McpError::Protocol(format!(
            "did not receive matching jsonrpc response id={id}"
        )))
    }

    async fn write_message(&self, message: &Value) -> Result<(), McpError> {
        let payload = serde_json::to_vec(message)
            .map_err(|err| McpError::Protocol(format!("failed to serialize request: {err}")))?;
        let header = format!("Content-Length: {}\r\n\r\n", payload.len());
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|err| McpError::Io(err.to_string()))?;
        stdin
            .write_all(&payload)
            .await
            .map_err(|err| McpError::Io(err.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|err| McpError::Io(err.to_string()))?;
        Ok(())
    }

    async fn read_message(&self) -> Result<Value, McpError> {
        let mut stdout = self.stdout.lock().await;
        match read_jsonrpc_frame(&mut *stdout).await {
            Ok(value) => Ok(value),
            Err(err) => {
                let mut child = self.child.lock().await;
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(McpError::Process(format!(
                        "stdio mcp server exited: {status}"
                    )));
                }
                Err(err)
            }
        }
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
mod tests {
    use super::*;

    use std::process::Command as ProcessCommand;

    use tokio::io::BufReader;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn streamable_http_initializes_and_lists_tools() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/mcp");

        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.expect("read");
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let request_lc = request.to_ascii_lowercase();

                if step == 0 {
                    assert!(request_lc.contains("\r\naccept:"), "request={request}");
                    assert!(request_lc.contains("application/json"), "request={request}");
                    assert!(
                        request_lc.contains("text/event-stream"),
                        "request={request}"
                    );
                    assert!(request.contains("\"method\":\"initialize\""));
                    let body = serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": 1,
                        "result": {
                            "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "stub", "version": "0"}
                        }
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: sess-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket.write_all(response.as_bytes()).await.expect("write");
                } else if step == 1 {
                    // notifications/initialized
                    assert!(
                        request_lc.contains("mcp-session-id: sess-1"),
                        "request={request}"
                    );
                    let response =
                        "HTTP/1.1 202 Accepted\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
                    socket.write_all(response.as_bytes()).await.expect("write");
                } else {
                    assert!(
                        request_lc.contains("mcp-session-id: sess-1"),
                        "request={request}"
                    );
                    assert!(
                        request_lc.contains("mcp-protocol-version: 2025-11-25"),
                        "request={request}"
                    );
                    assert!(request.contains("\"method\":\"tools/list\""));
                    let body = serde_json::json!({
                        "jsonrpc":"2.0",
                        "id": 2,
                        "result": {
                            "tools": [
                                {
                                    "name": "hello",
                                    "description": "hi",
                                    "inputSchema": {"type":"object","additionalProperties":false}
                                }
                            ]
                        }
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket.write_all(response.as_bytes()).await.expect("write");
                }
            }
        });

        let session = McpHttpSession::connect(&url, None).await.expect("connect");
        assert_eq!(session.session_id(), Some("sess-1"));
        let tools = session.list_tools().await.expect("list_tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "hello");

        server.await.expect("server");
    }

    #[tokio::test]
    async fn framed_stdio_parser_reads_jsonrpc_payload() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {"ok": true}
        })
        .to_string();
        let framed = format!(
            "Content-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            payload.len(),
            payload
        );

        tokio::spawn(async move {
            writer
                .write_all(framed.as_bytes())
                .await
                .expect("write frame");
        });

        let mut reader = BufReader::new(reader);
        let value = read_jsonrpc_frame(&mut reader).await.expect("parse frame");
        assert_eq!(value.get("jsonrpc").and_then(Value::as_str), Some("2.0"));
        assert_eq!(value.get("id").and_then(Value::as_u64), Some(7));
    }

    #[test]
    fn matching_id_accepts_number_and_string() {
        assert!(is_matching_id(Some(&serde_json::json!(3)), 3));
        assert!(is_matching_id(Some(&serde_json::json!("3")), 3));
        assert!(!is_matching_id(Some(&serde_json::json!("abc")), 3));
    }

    #[tokio::test]
    async fn stdio_session_round_trip_for_tools_and_resources() {
        if ProcessCommand::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipping stdio MCP round-trip test: python3 not available");
            return;
        }

        let script = r#"
import json
import os
import sys

def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        name, value = line.decode("utf-8").split(":", 1)
        headers[name.strip().lower()] = value.strip()
    length = int(headers.get("content-length", "0"))
    payload = sys.stdin.buffer.read(length)
    return json.loads(payload.decode("utf-8"))

def send_message(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

if os.getenv("MCP_AUTH_TOKEN") != "token-123":
    sys.exit(1)

message = read_message()
if message is None or message.get("method") != "initialize":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "protocolVersion": "2025-11-25",
        "capabilities": {"tools": {}, "resources": {}},
        "serverInfo": {"name": "stub", "version": "0"}
    }
})

message = read_message()
if message is None or message.get("method") != "notifications/initialized":
    sys.exit(1)

message = read_message()
if message is None or message.get("method") != "tools/list":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "tools": [
            {
                "name": "hello",
                "description": "hi",
                "inputSchema": {"type": "object"}
            }
        ]
    }
})

message = read_message()
if message is None or message.get("method") != "resources/read":
    sys.exit(1)
uri = message.get("params", {}).get("uri")
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "contents": [
            {
                "uri": uri,
                "mimeType": "text/plain",
                "text": "from-stdio"
            }
        ]
    }
})

message = read_message()
if message is None or message.get("method") != "tools/call":
    sys.exit(1)
send_message({
    "jsonrpc": "2.0",
    "id": message["id"],
    "result": {
        "isError": False,
        "content": [{"type": "text", "text": "ok"}]
    }
})
"#;

        let args = vec!["-u".to_string(), "-c".to_string(), script.to_string()];
        let session = McpStdioSession::connect(
            "python3",
            &args,
            &BTreeMap::new(),
            Some("token-123".to_string()),
        )
        .await
        .expect("connect stdio session");

        let tools = session.list_tools().await.expect("list tools over stdio");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "hello");

        let resource = session
            .read_resource("file:///tmp/demo")
            .await
            .expect("read resource over stdio");
        assert!(
            resource.to_string().contains("from-stdio"),
            "resource={resource}"
        );

        let result = session
            .call_tool("hello", serde_json::json!({"name": "world"}))
            .await
            .expect("call tool over stdio");
        assert!(result.to_string().contains("\"isError\":false"));
    }
}
