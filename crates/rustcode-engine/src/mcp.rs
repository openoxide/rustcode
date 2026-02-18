use std::collections::BTreeMap;

use rustcode_llm::ToolSpec;
use rustcode_mcp::{McpError, McpHttpSession, McpStdioSession};
use serde_json::Value;

const MCP_LIST_RESOURCES_TOOL: &str = "__resources_list";
const MCP_READ_RESOURCE_TOOL: &str = "__resources_read";

enum McpSession {
    Http(McpHttpSession),
    Stdio(McpStdioSession),
}

impl McpSession {
    async fn list_tools(&self) -> Result<Vec<rustcode_mcp::McpTool>, McpError> {
        match self {
            Self::Http(session) => session.list_tools().await,
            Self::Stdio(session) => session.list_tools().await,
        }
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        match self {
            Self::Http(session) => session.call_tool(name, arguments).await,
            Self::Stdio(session) => session.call_tool(name, arguments).await,
        }
    }

    async fn list_resources(&self) -> Result<Vec<rustcode_mcp::McpResource>, McpError> {
        match self {
            Self::Http(session) => session.list_resources().await,
            Self::Stdio(session) => session.list_resources().await,
        }
    }

    async fn read_resource(&self, uri: &str) -> Result<Value, McpError> {
        match self {
            Self::Http(session) => session.read_resource(uri).await,
            Self::Stdio(session) => session.read_resource(uri).await,
        }
    }

    fn protocol_version(&self) -> &str {
        match self {
            Self::Http(session) => session.protocol_version(),
            Self::Stdio(session) => session.protocol_version(),
        }
    }
}

/// Manages connections to MCP servers and exposes their tools.
pub struct McpRegistry {
    /// Map from server name to connected session.
    sessions: BTreeMap<String, McpSession>,
}

impl McpRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
        }
    }

    /// Connect to an MCP server and store the session under the given name.
    pub async fn connect(
        &mut self,
        name: String,
        endpoint: &str,
        bearer_token: Option<String>,
    ) -> Result<(), McpError> {
        let session = McpHttpSession::connect(endpoint, bearer_token).await?;
        self.sessions.insert(name, McpSession::Http(session));
        Ok(())
    }

    /// Spawn a stdio MCP server and store the session under the given name.
    pub async fn connect_stdio(
        &mut self,
        name: String,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        bearer_token: Option<String>,
    ) -> Result<(), McpError> {
        let session = McpStdioSession::connect(command, args, env, bearer_token).await?;
        self.sessions.insert(name, McpSession::Stdio(session));
        Ok(())
    }

    /// Returns true if any MCP servers are connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.sessions.is_empty()
    }

    /// Collect tool specs from all connected MCP servers.
    /// Tools are namespaced as `mcp:<server>:<tool>`.
    pub async fn tool_specs(&self) -> Result<Vec<(String, ToolSpec)>, McpError> {
        let mut specs = Vec::new();
        for (server_name, session) in &self.sessions {
            let tools = session.list_tools().await?;
            for tool in tools {
                let namespaced_name = format!("mcp:{server_name}:{}", tool.name);
                let spec = ToolSpec {
                    name: namespaced_name.clone(),
                    description: tool.description.unwrap_or_else(|| tool.name.clone()),
                    parameters: sanitize_mcp_schema(tool.input_schema),
                };
                specs.push((namespaced_name, spec));
            }

            if session.list_resources().await.is_ok() {
                let list_name = format!("mcp:{server_name}:{MCP_LIST_RESOURCES_TOOL}");
                specs.push((
                    list_name.clone(),
                    ToolSpec {
                        name: list_name,
                        description: format!(
                            "List resources exposed by MCP server `{server_name}`"
                        ),
                        parameters: serde_json::json!({
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {}
                        }),
                    },
                ));

                let read_name = format!("mcp:{server_name}:{MCP_READ_RESOURCE_TOOL}");
                specs.push((
                    read_name.clone(),
                    ToolSpec {
                        name: read_name,
                        description: format!(
                            "Read one MCP resource from server `{server_name}` by URI"
                        ),
                        parameters: serde_json::json!({
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["uri"],
                            "properties": {
                                "uri": {
                                    "type": "string",
                                    "description": "Resource URI from resources/list"
                                }
                            }
                        }),
                    },
                ));
            }
        }
        Ok(specs)
    }

    /// Call an MCP tool by its full namespaced name (e.g., `mcp:github:search_repos`).
    /// Returns the tool result as a JSON string.
    pub async fn call_tool(&self, full_name: &str, arguments: Value) -> Result<String, McpError> {
        let parts: Vec<&str> = full_name.splitn(3, ':').collect();
        if parts.len() != 3 || parts[0] != "mcp" {
            return Err(McpError::Protocol(format!(
                "invalid mcp tool name: {full_name}"
            )));
        }
        let server_name = parts[1];
        let tool_name = parts[2];

        let session = self.sessions.get(server_name).ok_or_else(|| {
            McpError::Protocol(format!("mcp server not connected: {server_name}"))
        })?;

        let result = if tool_name == MCP_LIST_RESOURCES_TOOL {
            let resources = session.list_resources().await?;
            serde_json::to_value(resources)
                .map_err(|err| McpError::Protocol(format!("failed to encode resources: {err}")))?
        } else if tool_name == MCP_READ_RESOURCE_TOOL {
            let uri = arguments
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    McpError::Protocol(
                        "mcp resource read requires string argument `uri`".to_string(),
                    )
                })?;
            session.read_resource(uri).await?
        } else {
            session.call_tool(tool_name, arguments).await?
        };
        serde_json::to_string(&result)
            .map_err(|err| McpError::Protocol(format!("failed to serialize result: {err}")))
    }

    /// Returns a mapping from server name to connected session info.
    #[must_use]
    pub fn connected_servers(&self) -> BTreeMap<String, String> {
        self.sessions
            .iter()
            .map(|(name, session)| {
                let version = session.protocol_version().to_string();
                (name.clone(), version)
            })
            .collect()
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitize MCP tool input schema for provider compatibility.
/// Currently removes `additionalProperties: false` for Google compatibility.
fn sanitize_mcp_schema(schema: Value) -> Value {
    match schema {
        Value::Object(mut obj) => {
            // Google Generative AI doesn't accept `additionalProperties: false`.
            if let Some(Value::Bool(false)) = obj.get("additionalProperties") {
                obj.remove("additionalProperties");
            }
            // Recursively sanitize nested schemas.
            if let Some(props) = obj.get_mut("properties") {
                *props = sanitize_mcp_schema(props.clone());
            }
            if let Some(items) = obj.get_mut("items") {
                *items = sanitize_mcp_schema(items.clone());
            }
            Value::Object(obj)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sanitize_mcp_schema).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn sanitize_removes_additional_properties_false() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false
        });
        let sanitized = sanitize_mcp_schema(schema);
        assert!(!sanitized
            .as_object()
            .unwrap()
            .contains_key("additionalProperties"));
    }

    #[test]
    fn sanitize_preserves_additional_properties_true() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": true
        });
        let sanitized = sanitize_mcp_schema(schema);
        assert!(sanitized
            .as_object()
            .unwrap()
            .contains_key("additionalProperties"));
    }

    #[test]
    fn parse_resource_read_argument_requires_uri_string() {
        let value = serde_json::json!({"uri": "mcp://resource/1"});
        assert_eq!(
            value.get("uri").and_then(Value::as_str),
            Some("mcp://resource/1")
        );
        let invalid = serde_json::json!({"uri": 1});
        assert!(invalid.get("uri").and_then(Value::as_str).is_none());
    }

    #[tokio::test]
    async fn tool_specs_include_resource_helpers_when_resources_supported() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            for step in 0..4 {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.expect("read");
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if step == 0 {
                    assert!(request.contains("\"method\":\"initialize\""));
                    let body = serde_json::json!({
                        "jsonrpc":"2.0",
                        "id":1,
                        "result": {
                            "protocolVersion":"2025-11-25",
                            "capabilities":{},
                            "serverInfo":{"name":"stub","version":"0"}
                        }
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: sess-res\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket.write_all(response.as_bytes()).await.expect("write");
                } else if step == 1 {
                    assert!(request.contains("notifications/initialized"));
                    let response =
                        "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    socket.write_all(response.as_bytes()).await.expect("write");
                } else if step == 2 {
                    assert!(request.contains("\"method\":\"tools/list\""));
                    let body = serde_json::json!({
                        "jsonrpc":"2.0",
                        "id":2,
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
                } else {
                    assert!(request.contains("\"method\":\"resources/list\""));
                    let body = serde_json::json!({
                        "jsonrpc":"2.0",
                        "id":3,
                        "result": {
                            "resources": [
                                {
                                    "uri": "file:///tmp/demo",
                                    "name": "demo"
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

        let mut registry = McpRegistry::new();
        registry
            .connect("demo".to_string(), &format!("http://{addr}/mcp"), None)
            .await
            .expect("connect mcp registry");

        let specs = registry.tool_specs().await.expect("tool specs");
        let names = specs.into_iter().map(|(name, _)| name).collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "mcp:demo:hello"));
        assert!(names.iter().any(|name| name == "mcp:demo:__resources_list"));
        assert!(names.iter().any(|name| name == "mcp:demo:__resources_read"));

        server.await.expect("server join");
    }
}
