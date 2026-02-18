use std::collections::BTreeMap;

use rustcode_mcp::{McpError, McpHttpSession};
use rustcode_llm::ToolSpec;
use serde_json::Value;

/// Manages connections to MCP servers and exposes their tools.
pub struct McpRegistry {
    /// Map from server name to connected session.
    sessions: BTreeMap<String, McpHttpSession>,
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
        self.sessions.insert(name, session);
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

        let session = self
            .sessions
            .get(server_name)
            .ok_or_else(|| McpError::Protocol(format!("mcp server not connected: {server_name}")))?;

        let result = session.call_tool(tool_name, arguments).await?;
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
        Value::Array(arr) => Value::Array(
            arr.into_iter().map(sanitize_mcp_schema).collect()
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!sanitized.as_object().unwrap().contains_key("additionalProperties"));
    }

    #[test]
    fn sanitize_preserves_additional_properties_true() {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": true
        });
        let sanitized = sanitize_mcp_schema(schema);
        assert!(sanitized.as_object().unwrap().contains_key("additionalProperties"));
    }
}
