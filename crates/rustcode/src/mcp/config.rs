use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use rustcode_config::{ConfigEditScope, ConfigLoader, ConfigSources};
use rustcode_core::config::McpServerConfig as CoreMcpServerConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfigEntry {
    pub name: String,
    pub configured: bool,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub oauth_enabled: bool,
    pub client_id: Option<String>,
    pub client_secret_env: Option<String>,
}

impl McpConfigEntry {
    pub fn transport_kind(&self) -> &'static str {
        if self.command.is_some() {
            "stdio"
        } else {
            "http"
        }
    }

    pub fn oauth_client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    pub fn oauth_client_secret_env(&self) -> Option<&str> {
        self.client_secret_env.as_deref()
    }
}

impl From<(String, &CoreMcpServerConfig)> for McpConfigEntry {
    fn from((name, value): (String, &CoreMcpServerConfig)) -> Self {
        Self {
            name,
            configured: true,
            url: value.url.clone(),
            command: value.command.clone(),
            args: value.args.clone(),
            env: value.env.clone(),
            oauth_enabled: value.oauth.enabled,
            client_id: value.oauth.client_id.clone(),
            client_secret_env: value.oauth.client_secret_env.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SidecarServerRaw {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    oauth: Option<SidecarOauthRaw>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret_env: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SidecarOauthRaw {
    Bool(bool),
    Object {
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        client_secret_env: Option<String>,
    },
}

impl McpConfigEntry {
    fn from_sidecar(name: String, raw: SidecarServerRaw) -> Self {
        let (oauth_enabled, oauth_client_id, oauth_client_secret_env) = match raw.oauth {
            Some(SidecarOauthRaw::Bool(enabled)) => (enabled, raw.client_id, raw.client_secret_env),
            Some(SidecarOauthRaw::Object {
                enabled,
                client_id,
                client_secret_env,
            }) => (
                enabled.unwrap_or(true),
                client_id.or(raw.client_id),
                client_secret_env.or(raw.client_secret_env),
            ),
            None => (true, raw.client_id, raw.client_secret_env),
        };

        Self {
            name,
            configured: true,
            url: raw.url,
            command: raw.command,
            args: raw.args,
            env: raw.env,
            oauth_enabled,
            client_id: oauth_client_id,
            client_secret_env: oauth_client_secret_env,
        }
    }
}

pub fn resolve_configured_mcp_servers(
    trust_project_config: bool,
) -> Result<BTreeMap<String, McpConfigEntry>> {
    let mut servers = BTreeMap::new();

    let cwd = std::env::current_dir()?;
    let mut sources = ConfigSources::new(cwd);
    sources.trust_project = trust_project_config;

    if let Ok(config) = ConfigLoader::load(&sources) {
        for (name, server) in config.mcp_servers {
            servers.insert(name.clone(), McpConfigEntry::from((name, &server)));
        }
    }

    if let Some(path) = resolve_mcp_servers_path() {
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(sidecar) =
                serde_json::from_str::<BTreeMap<String, SidecarServerRaw>>(&contents)
            {
                for (name, raw) in sidecar {
                    servers
                        .entry(name.clone())
                        .or_insert_with(|| McpConfigEntry::from_sidecar(name, raw));
                }
            }
        }
    }

    Ok(servers)
}

fn resolve_mcp_servers_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUSTCODE_MCP_SERVERS_PATH") {
        return Some(PathBuf::from(path));
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let path = PathBuf::from(home).join(".config/rustcode/mcp_servers.json");
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

pub fn parse_mcp_edit_scope(raw: &str) -> Result<ConfigEditScope> {
    match raw {
        "user" => Ok(ConfigEditScope::User),
        "project" => Ok(ConfigEditScope::Project),
        _ => anyhow::bail!("invalid scope: {raw} (expected user or project)"),
    }
}

pub fn parse_mcp_env_assignments(raw_items: &[String]) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for item in raw_items {
        if let Some((key, value)) = item.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        } else {
            anyhow::bail!("invalid env assignment: {item} (expected KEY=VAL)");
        }
    }
    Ok(map)
}
