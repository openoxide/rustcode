use std::time::Duration;

use anyhow::{Context, Result};
use rustcode_auth::{
    complete_mcp_browser_oauth_flow, discover_mcp_oauth, start_mcp_browser_oauth_flow, AuthStore,
};

use crate::cli::McpLoginMethod;
use crate::utils::write_stdout_line;

use super::config::McpConfigEntry;
use super::{mcp_store_key, resolve_mcp_login_url, resolve_mcp_oauth_client_id, resolve_mcp_oauth_client_secret};

pub(super) struct LoginRequest {
    pub name: String,
    pub method: Option<String>,
    pub url: Option<String>,
    pub from_env: Option<String>,
    pub scopes: Vec<String>,
    pub client_id: Option<String>,
    pub client_secret_env: Option<String>,
    pub no_wait: bool,
    pub timeout_secs: u64,
    pub oauth_port: u16,
}

pub(super) async fn handle_mcp_login(
    store: &AuthStore,
    req: LoginRequest,
    json_output: bool,
    trust_project_config: bool,
) -> Result<()> {
    if req.from_env.is_some() && req.method.as_deref() == Some("oauth_browser") {
        anyhow::bail!("--from-env cannot be combined with --method oauth_browser");
    }

    let servers = super::config::resolve_configured_mcp_servers(trust_project_config)?;
    let config_entry = servers.get(&req.name);
    let method = resolve_mcp_login_method(req.method.as_deref())?;

    match method {
        McpLoginMethod::ApiKey => {
            handle_api_key_login(store, &req, config_entry, json_output).await?;
        }
        McpLoginMethod::OAuthBrowser => {
            handle_oauth_browser_login(store, &req, config_entry, json_output).await?;
        }
    }

    Ok(())
}

async fn handle_api_key_login(
    store: &AuthStore,
    req: &LoginRequest,
    config_entry: Option<&McpConfigEntry>,
    json_output: bool,
) -> Result<()> {
    if let Some(env_var) = &req.from_env {
        let value = std::env::var(env_var).context(format!("env var {env_var} not set"))?;
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

    let url = resolve_mcp_login_url(req.url.clone(), config_entry)?;
    let discovery = discover_mcp_oauth(&url)
        .await
        .context("failed to discover mcp oauth")?;

    emit_oauth_discovered(&req.name, &url, "token_import", &req.scopes, &discovery, json_output)?;

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

    Ok(())
}

async fn handle_oauth_browser_login(
    store: &AuthStore,
    req: &LoginRequest,
    config_entry: Option<&McpConfigEntry>,
    json_output: bool,
) -> Result<()> {
    let url = resolve_mcp_login_url(req.url.clone(), config_entry)?;
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

    let client_id = resolve_mcp_oauth_client_id(req.client_id.clone(), config_entry)?;
    let client_secret_env =
        resolve_mcp_oauth_client_secret(req.client_secret_env.clone(), config_entry);
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

fn resolve_mcp_login_method(raw: Option<&str>) -> Result<McpLoginMethod> {
    match raw {
        Some("api_key" | "token_import") | None => Ok(McpLoginMethod::ApiKey),
        Some("oauth_browser") => Ok(McpLoginMethod::OAuthBrowser),
        Some(other) => anyhow::bail!("invalid mcp login method: {other}"),
    }
}
