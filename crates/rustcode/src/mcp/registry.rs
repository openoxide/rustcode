use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rustcode_auth::{discover_mcp_oauth, AuthStore, StoredCredential};
use rustcode_core::config::ResolvedConfig;
use rustcode_engine::mcp::McpRegistry;

use crate::utils::write_stdout_line;

use super::config::{resolve_configured_mcp_servers, McpConfigEntry};

/// Build an `McpRegistry` from the resolved configuration, connecting all configured servers.
pub async fn build_mcp_registry(config: &ResolvedConfig) -> Result<McpRegistry> {
    let mut registry = McpRegistry::new();
    let store = AuthStore::open_default();

    for (name, server) in &config.mcp_servers {
        let bearer = match store.get(&mcp_store_key(name))? {
            Some(StoredCredential::ApiKey { key, .. }) => Some(key),
            _ => None,
        };

        if let Some(url) = &server.url {
            registry
                .connect(name.clone(), url, bearer)
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        } else if let Some(command) = &server.command {
            registry
                .connect_stdio(name.clone(), command, &server.args, &server.env, bearer)
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        }
    }

    Ok(registry)
}

/// List all configured MCP servers and their credential status.
pub(super) fn handle_mcp_list(
    store: &AuthStore,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    let mut servers = resolve_configured_mcp_servers(trust_project_config)?;
    for provider in store.providers()? {
        if let Some(name) = provider.strip_prefix("mcp:") {
            servers.entry(name.to_string()).or_insert(McpConfigEntry {
                name: name.to_string(),
                configured: false,
                url: None,
                command: None,
                args: Vec::new(),
                env: BTreeMap::new(),
                oauth_enabled: false,
                client_id: None,
                client_secret_env: None,
            });
        }
    }

    if json_output {
        let mut rows = Vec::with_capacity(servers.len());
        for (name, entry) in servers {
            rows.push(serde_json::json!({
                "name": name,
                "configured": entry.configured,
                "url": entry.url,
                "transport": entry.transport_kind(),
                "oauth_enabled": entry.oauth_enabled,
                "credential": credential_label(store, &name)?,
                "command": entry.command,
                "args": entry.args,
            }));
        }
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "mcp.list",
            "servers": rows,
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("servers={}", servers.len()))?;
        for (name, _entry) in servers {
            write_stdout_line(&format!(
                "name={name}\tcredential={}",
                credential_label(store, &name)?
            ))?;
        }
    }

    Ok(())
}

/// Show status (including OAuth discovery) for one or all configured MCP servers.
pub(super) async fn handle_mcp_status(
    store: &AuthStore,
    name: Option<&str>,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    let servers = resolve_configured_mcp_servers(trust_project_config)?;

    if json_output {
        handle_mcp_status_json(store, name, &servers).await?;
    } else if let Some(name) = name {
        let entry = servers
            .get(name)
            .with_context(|| format!("mcp server {name} not found in config"))?;
        write_stdout_line(&format!("name={name}"))?;
        write_stdout_line(&format!("transport={}", entry.transport_kind()))?;
        write_stdout_line(&format!("oauth_enabled={}", entry.oauth_enabled))?;
        write_stdout_line(&format!("credential={}", credential_label(store, name)?))?;
    } else {
        write_stdout_line(&format!("servers={}", servers.len()))?;
        for (name, _) in servers {
            write_stdout_line(&format!(
                "name={name}\tcredential={}",
                credential_label(store, &name)?
            ))?;
        }
    }

    Ok(())
}

async fn handle_mcp_status_json(
    store: &AuthStore,
    name: Option<&str>,
    servers: &BTreeMap<String, McpConfigEntry>,
) -> Result<()> {
    if let Some(name) = name {
        let entry = servers
            .get(name)
            .with_context(|| format!("mcp server {name} not found in config"))?;
        let (oauth_supported, discovery) = mcp_discovery_status(entry).await;
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "mcp.status",
            "server": {
                "name": name,
                "configured": entry.configured,
                "url": entry.url,
                "transport": entry.transport_kind(),
                "oauth_enabled": entry.oauth_enabled,
                "oauth_supported": oauth_supported,
                "discovery": discovery,
                "credential": credential_label(store, name)?,
            }
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        let mut rows = Vec::new();
        for (name, entry) in servers {
            let (oauth_supported, discovery) = mcp_discovery_status(entry).await;
            rows.push(serde_json::json!({
                "name": name,
                "configured": entry.configured,
                "url": entry.url,
                "transport": entry.transport_kind(),
                "oauth_enabled": entry.oauth_enabled,
                "oauth_supported": oauth_supported,
                "discovery": discovery,
                "credential": credential_label(store, name)?,
            }));
        }
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "mcp.status",
            "servers": rows,
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    }

    Ok(())
}

async fn mcp_discovery_status(entry: &McpConfigEntry) -> (bool, serde_json::Value) {
    if !entry.oauth_enabled {
        return (false, serde_json::Value::Null);
    }

    let Some(url) = &entry.url else {
        return (false, serde_json::Value::Null);
    };

    match discover_mcp_oauth(url).await {
        Ok(discovery) => (
            discovery.supported,
            serde_json::json!({
                "metadata_url": discovery.metadata_url,
                "authorization_endpoint": discovery.authorization_endpoint,
                "token_endpoint": discovery.token_endpoint,
            }),
        ),
        Err(_) => (false, serde_json::Value::Null),
    }
}

/// Classify an MCP error by category for structured output.
pub fn classify_mcp_error(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("validation")
        || message.contains("requires")
        || message.contains("not found")
        || message.contains("cannot be combined")
        || message.contains("invalid")
    {
        "validation"
    } else if message.contains("network") || message.contains("dns") || message.contains("connect")
    {
        "network"
    } else {
        "provider"
    }
}

pub(super) fn mcp_store_key(name: &str) -> String {
    format!("mcp:{name}")
}

pub(super) fn credential_label(store: &AuthStore, name: &str) -> Result<&'static str> {
    match store.get(&mcp_store_key(name))? {
        Some(StoredCredential::ApiKey { .. }) => Ok("stored:api_key"),
        _ => Ok("none"),
    }
}
