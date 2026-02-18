use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use rustcode_auth::{
    complete_mcp_browser_oauth_flow, discover_mcp_oauth, start_mcp_browser_oauth_flow, AuthStore,
    StoredCredential,
};
use rustcode_config::{edit_mcp_server, remove_mcp_server};
use rustcode_core::config::ResolvedConfig;
use rustcode_engine::mcp::McpRegistry;

use crate::cli::{McpCommand, McpLoginMethod};
use crate::utils::write_stdout_line;

mod config;

use config::{
    McpConfigEntry, parse_mcp_edit_scope, parse_mcp_env_assignments,
    resolve_configured_mcp_servers,
};

pub async fn handle_mcp_command(
    command: McpCommand,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    let store = AuthStore::open_default();

    match command {
        McpCommand::List => handle_mcp_list(&store, json_output, trust_project_config)?,
        McpCommand::Status { name } => {
            handle_mcp_status(&store, name.as_deref(), json_output, trust_project_config).await?
        }
        McpCommand::Login {
            name,
            method,
            url,
            from_env,
            scopes,
            client_id,
            client_secret_env,
            no_wait,
            timeout_secs,
            oauth_port,
        } => {
            handle_mcp_login(
                &store,
                LoginRequest {
                    name,
                    method,
                    url,
                    from_env,
                    scopes,
                    client_id,
                    client_secret_env,
                    no_wait,
                    timeout_secs,
                    oauth_port,
                },
                json_output,
                trust_project_config,
            )
            .await?
        }
        McpCommand::Logout { name } => {
            let removed = store.remove(&mcp_store_key(&name))?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.logout",
                    "name": name,
                    "removed": removed,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else if removed {
                write_stdout_line(&format!("removed mcp credential for name={name}"))?;
            } else {
                write_stdout_line(&format!("no mcp credential for name={name}"))?;
            }
        }
        McpCommand::Add {
            name,
            url,
            command,
            args,
            env,
            oauth,
            client_id,
            client_secret_env,
            scope,
        } => {
            let edit_scope = parse_mcp_edit_scope(&scope)?;
            let cwd = std::env::current_dir()?;
            let mcp_config = rustcode_core::config::McpServerConfig {
                url,
                command: command.clone(),
                args: args.clone(),
                env: parse_mcp_env_assignments(&env)?,
                oauth: rustcode_core::config::McpOAuthConfig {
                    enabled: oauth.as_deref().is_none_or(|value| value == "on"),
                    client_id,
                    client_secret_env,
                },
            };

            edit_mcp_server(edit_scope, &cwd, &name, &mcp_config)
                .context("failed to add mcp server to config")?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.add",
                    "name": name,
                    "scope": scope,
                    "transport": if command.is_some() { "stdio" } else { "http" },
                    "command_name": command,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("added mcp server {name} to {scope} config"))?;
            }
        }
        McpCommand::Remove { name, scope } => {
            let edit_scope = parse_mcp_edit_scope(&scope)?;
            let cwd = std::env::current_dir()?;
            remove_mcp_server(edit_scope, &cwd, &name)
                .context("failed to remove mcp server from config")?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.remove",
                    "name": name,
                    "scope": scope,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("removed mcp server {name} from {scope} config"))?;
            }
        }
        McpCommand::Get { name } => {
            anyhow::bail!("mcp get {name}: not implemented")
        }
    }

    Ok(())
}

struct LoginRequest {
    name: String,
    method: Option<String>,
    url: Option<String>,
    from_env: Option<String>,
    scopes: Vec<String>,
    client_id: Option<String>,
    client_secret_env: Option<String>,
    no_wait: bool,
    timeout_secs: u64,
    oauth_port: u16,
}

async fn handle_mcp_login(
    store: &AuthStore,
    req: LoginRequest,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    if req.from_env.is_some() && req.method.as_deref() == Some("oauth_browser") {
        anyhow::bail!("--from-env cannot be combined with --method oauth_browser");
    }

    let servers = resolve_configured_mcp_servers(trust_project_config)?;
    let config_entry = servers.get(&req.name);
    let method = resolve_mcp_login_method(req.method.as_deref())?;

    match method {
        McpLoginMethod::ApiKey => {
            if let Some(env_var) = req.from_env {
                let value =
                    std::env::var(&env_var).context(format!("env var {env_var} not set"))?;
                store.set_api_key(&mcp_store_key(&req.name), &value)?;

                if json_output {
                    let payload = serde_json::json!({
                        "schema_version": 1,
                        "command": "mcp.login",
                        "name": req.name,
                        "stage": "authorized",
                        "credential": "stored:api_key",
                    });
                    write_stdout_line(&serde_json::to_string(&payload)?)?;
                } else {
                    write_stdout_line(&format!("stored mcp credential for name={}", req.name))?;
                }
                return Ok(());
            }

            if req.scopes.is_empty() {
                anyhow::bail!(
                    "mcp login requires either --from-env <ENV_VAR> or --url <MCP_URL> with --scopes <SCOPE,SCOPE>"
                );
            }

            let url = resolve_mcp_login_url(req.url, config_entry)?;
            let discovery = discover_mcp_oauth(&url)
                .await
                .context("failed to discover mcp oauth")?;

            emit_oauth_discovered(
                &req.name,
                &url,
                "token_import",
                &req.scopes,
                &discovery,
                json_output,
            )?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.login",
                    "name": req.name,
                    "method": "token_import",
                    "stage": "awaiting_token_import",
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line("status=awaiting_token_import")?;
            }
        }
        McpLoginMethod::OAuthBrowser => {
            let url = resolve_mcp_login_url(req.url, config_entry)?;
            let discovery = discover_mcp_oauth(&url)
                .await
                .context("failed to discover mcp oauth")?;

            emit_oauth_discovered(
                &req.name,
                &url,
                "oauth_browser",
                &req.scopes,
                &discovery,
                json_output,
            )?;

            let client_id = resolve_mcp_oauth_client_id(req.client_id, config_entry)?;
            let client_secret_env =
                resolve_mcp_oauth_client_secret(req.client_secret_env, config_entry);
            let client_secret = if let Some(env_name) = client_secret_env {
                Some(
                    std::env::var(&env_name)
                        .context(format!("mcp client secret env {env_name} not set"))?,
                )
            } else {
                None
            };

            let flow = start_mcp_browser_oauth_flow(
                &req.name,
                &url,
                &discovery,
                &client_id,
                req.oauth_port,
                &req.scopes,
            )
            .context("failed to start mcp browser oauth flow")?;

            if json_output {
                let challenge = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.login",
                    "name": req.name,
                    "method": "oauth_browser",
                    "stage": "challenge",
                    "oauth_port": req.oauth_port,
                    "authorize_url": flow.authorize_url,
                    "redirect_uri": flow.redirect_uri,
                });
                write_stdout_line(&serde_json::to_string(&challenge)?)?;

                let awaiting = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.login",
                    "name": req.name,
                    "method": "oauth_browser",
                    "stage": "awaiting_browser_callback",
                });
                write_stdout_line(&serde_json::to_string(&awaiting)?)?;
            } else {
                write_stdout_line("method=oauth_browser")?;
                write_stdout_line(&format!("authorize_url={}", flow.authorize_url))?;
                write_stdout_line(&format!("redirect_uri={}", flow.redirect_uri))?;
                write_stdout_line("status=awaiting_browser_callback")?;
            }

            if req.no_wait {
                return Ok(());
            }

            let credential = complete_mcp_browser_oauth_flow(
                &flow,
                Duration::from_secs(req.timeout_secs),
                client_secret.as_deref(),
            )
            .await
            .context("failed to complete mcp browser oauth flow")?;

            store.set_api_key(&mcp_store_key(&req.name), &credential.access_token)?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.login",
                    "name": req.name,
                    "stage": "authorized",
                    "credential": "stored:api_key",
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line("status=authorized")?;
            }
        }
    }

    Ok(())
}

fn emit_oauth_discovered(
    name: &str,
    url: &str,
    method: &str,
    scopes: &[String],
    discovery: &rustcode_auth::McpOAuthDiscovery,
    json_output: bool,
) -> Result<()> {
    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "mcp.login",
            "name": name,
            "url": url,
            "method": method,
            "stage": "oauth_discovered",
            "authorization_endpoint": discovery.authorization_endpoint,
            "token_endpoint": discovery.token_endpoint,
            "scopes": scopes,
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("name={name}"))?;
        write_stdout_line(&format!("url={url}"))?;
        write_stdout_line("status=oauth_discovered")?;
    }

    Ok(())
}

fn handle_mcp_list(store: &AuthStore, json_output: bool, trust_project_config: bool) -> Result<()> {
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

async fn handle_mcp_status(
    store: &AuthStore,
    name: Option<&str>,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    let servers = resolve_configured_mcp_servers(trust_project_config)?;

    if json_output {
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
                let (oauth_supported, discovery) = mcp_discovery_status(&entry).await;
                rows.push(serde_json::json!({
                    "name": name,
                    "configured": entry.configured,
                    "url": entry.url,
                    "transport": entry.transport_kind(),
                    "oauth_enabled": entry.oauth_enabled,
                    "oauth_supported": oauth_supported,
                    "discovery": discovery,
                    "credential": credential_label(store, &name)?,
                }));
            }
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "mcp.status",
                "servers": rows,
            });
            write_stdout_line(&serde_json::to_string(&payload)?)?;
        }
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
            write_stdout_line(&format!("name={name}\tcredential={}", credential_label(store, &name)?))?;
        }
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

fn resolve_mcp_login_method(raw: Option<&str>) -> Result<McpLoginMethod> {
    match raw {
        Some("api_key" | "token_import") | None => Ok(McpLoginMethod::ApiKey),
        Some("oauth_browser") => Ok(McpLoginMethod::OAuthBrowser),
        Some(other) => anyhow::bail!("invalid mcp login method: {other}"),
    }
}

fn resolve_mcp_login_url(
    explicit: Option<String>,
    config: Option<&McpConfigEntry>,
) -> Result<String> {
    if let Some(url) = explicit {
        return Ok(url);
    }

    if let Some(config) = config {
        if let Some(url) = &config.url {
            return Ok(url.clone());
        }
    }

    anyhow::bail!("mcp login requires --url or a configured server URL")
}

fn resolve_mcp_oauth_client_id(
    explicit: Option<String>,
    config: Option<&McpConfigEntry>,
) -> Result<String> {
    if let Some(client_id) = explicit {
        return Ok(client_id);
    }

    if let Some(config) = config {
        if let Some(client_id) = config.oauth_client_id() {
            return Ok(client_id.to_string());
        }
    }

    anyhow::bail!("mcp oauth login requires --client-id or a configured client_id")
}

fn resolve_mcp_oauth_client_secret(
    explicit: Option<String>,
    config: Option<&McpConfigEntry>,
) -> Option<String> {
    explicit.or_else(|| config.and_then(|entry| entry.oauth_client_secret_env().map(ToString::to_string)))
}

fn mcp_store_key(name: &str) -> String {
    format!("mcp:{name}")
}

fn credential_label(store: &AuthStore, name: &str) -> Result<&'static str> {
    match store.get(&mcp_store_key(name))? {
        Some(StoredCredential::ApiKey { .. }) => Ok("stored:api_key"),
        _ => Ok("none"),
    }
}

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

pub fn classify_mcp_error(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("validation")
        || message.contains("requires")
        || message.contains("not found")
        || message.contains("cannot be combined")
        || message.contains("invalid")
    {
        "validation"
    } else if message.contains("network") || message.contains("dns") || message.contains("connect") {
        "network"
    } else {
        "provider"
    }
}
