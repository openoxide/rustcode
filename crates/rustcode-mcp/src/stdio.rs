use super::{McpStdioSession, BTreeMap, McpError, Command, Stdio, AsyncReadExt, Mutex, BufReader, DEFAULT_PROTOCOL_VERSION, AtomicU64, McpTool, Value, McpResource, Ordering, STDIO_RESPONSE_TIMEOUT, is_matching_id, AsyncWriteExt, read_jsonrpc_frame};

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
                .map(std::string::ToString::to_string);
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
                .map(std::string::ToString::to_string);
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
            Some(err) => Err(McpError::Protocol(format!("jsonrpc error: {err}"))),
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
                    return Err(McpError::Process(format!("stdio mcp server exited: {status}")));
                }
                Err(err)
            }
        }
    }
}
