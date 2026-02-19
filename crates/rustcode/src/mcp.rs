use anyhow::{Context, Result};
use rustcode_auth::AuthStore;
use rustcode_config::{edit_mcp_server, remove_mcp_server};

use crate::cli::McpCommand;
use crate::utils::write_stdout_line;

mod config;
mod login;
mod registry;

use config::{parse_mcp_edit_scope, parse_mcp_env_assignments, McpConfigEntry};

pub use registry::{build_mcp_registry, classify_mcp_error};

/// Dispatch an MCP subcommand.
pub async fn handle_mcp_command(
    command: McpCommand,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    let store = AuthStore::open_default();

    match command {
        McpCommand::List => {
            registry::handle_mcp_list(&store, json_output, trust_project_config)?;
        }
        McpCommand::Status { name } => {
            registry::handle_mcp_status(&store, name.as_deref(), json_output, trust_project_config)
                .await?;
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
            login::handle_mcp_login(
                &store,
                login::LoginRequest {
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
            .await?;
        }
        McpCommand::Logout { name } => {
            handle_mcp_logout(&store, &name, json_output)?;
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
            handle_mcp_add(
                &name,
                url,
                command,
                args,
                &env,
                oauth.as_deref(),
                client_id,
                client_secret_env,
                &scope,
                json_output,
            )?;
        }
        McpCommand::Remove { name, scope } => {
            handle_mcp_remove(&name, &scope, json_output)?;
        }
        McpCommand::Get { name } => {
            anyhow::bail!("mcp get {name}: not implemented");
        }
    }

    Ok(())
}

fn handle_mcp_logout(store: &AuthStore, name: &str, json_output: bool) -> Result<()> {
    let removed = store.remove(&mcp_store_key(name))?;
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
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_mcp_add(
    name: &str,
    url: Option<String>,
    command: Option<String>,
    args: Vec<String>,
    env: &[String],
    oauth: Option<&str>,
    client_id: Option<String>,
    client_secret_env: Option<String>,
    scope: &str,
    json_output: bool,
) -> Result<()> {
    let edit_scope = parse_mcp_edit_scope(scope)?;
    let cwd = std::env::current_dir()?;
    let mcp_config = rustcode_core::config::McpServerConfig {
        url,
        command: command.clone(),
        args: args.clone(),
        env: parse_mcp_env_assignments(env)?,
        oauth: rustcode_core::config::McpOAuthConfig {
            enabled: oauth.is_none_or(|value| value == "on"),
            client_id,
            client_secret_env,
        },
    };

    edit_mcp_server(edit_scope, &cwd, name, &mcp_config)
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
    Ok(())
}

fn handle_mcp_remove(name: &str, scope: &str, json_output: bool) -> Result<()> {
    let edit_scope = parse_mcp_edit_scope(scope)?;
    let cwd = std::env::current_dir()?;
    remove_mcp_server(edit_scope, &cwd, name).context("failed to remove mcp server from config")?;
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
    Ok(())
}

fn mcp_store_key(name: &str) -> String {
    format!("mcp:{name}")
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
    explicit.or_else(|| {
        config.and_then(|entry| entry.oauth_client_secret_env().map(ToString::to_string))
    })
}
