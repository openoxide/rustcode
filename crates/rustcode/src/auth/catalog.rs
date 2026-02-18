use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rustcode_auth::{methods_for_provider, AuthMethod};
use serde::{Deserialize, Serialize};

use crate::utils::write_stdout_line;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsProvider {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub auth: Option<AuthMethodsConfig>,
    #[serde(default)]
    pub models: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthMethodsConfig {
    #[serde(default)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthMethodRow {
    pub id: String,
    pub name: String,
    pub methods: Vec<AuthMethod>,
}

pub fn resolve_auth_method_rows() -> (Vec<AuthMethodRow>, Option<String>) {
    let mut rows = Vec::new();
    let mut warning = None;

    match load_models_index() {
        Ok(index) => {
            for (id, provider) in index {
                let methods = provider
                    .auth
                    .as_ref()
                    .map(|config| parse_auth_methods(&config.methods))
                    .filter(|methods| !methods.is_empty())
                    .unwrap_or_else(|| methods_for_provider(&id));
                rows.push(AuthMethodRow {
                    id,
                    name: provider.name,
                    methods,
                });
            }
        }
        Err(err) => {
            warning = Some(format!("models index unavailable: {err}"));
            for id in ["openai", "anthropic", "github-copilot", "gitlab"] {
                rows.push(AuthMethodRow {
                    id: id.to_string(),
                    name: fallback_provider_name(id).to_string(),
                    methods: methods_for_provider(id),
                });
            }
        }
    }

    rows.sort_by_cached_key(|row| (auth_login_priority(&row.id), row.name.clone()));
    (rows, warning)
}

pub fn list_auth_login_providers(json_output: bool) -> Result<()> {
    let (rows, warning) = resolve_auth_method_rows();

    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "auth.login",
            "warning": warning,
            "usage": {
                "api_key": "rustcode auth login <provider> --from-env <ENV_VAR>",
                "oauth": "rustcode auth login <provider> --method <oauth_device_code|oauth_browser>",
            },
            "providers": rows.iter().map(|row| serde_json::json!({
                "id": row.id,
                "name": row.name,
                "methods": row.methods.iter().map(|method| method.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line("usage_api_key=rustcode auth login <provider> --from-env <ENV_VAR>")?;
        write_stdout_line(
            "usage_oauth=rustcode auth login <provider> --method <oauth_device_code|oauth_browser>",
        )?;
        for row in rows {
            write_stdout_line(&format!(
                "provider={}\tname={}\tmethods={}",
                row.id,
                row.name,
                methods_as_pipe(&row.methods)
            ))?;
        }
    }

    Ok(())
}

pub fn list_auth_methods(json_output: bool) -> Result<()> {
    let (rows, warning) = resolve_auth_method_rows();

    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "command": "auth.methods",
            "warning": warning,
            "providers": rows.iter().map(|row| serde_json::json!({
                "id": row.id,
                "name": row.name,
                "methods": row.methods.iter().map(|method| method.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        write_stdout_line(&serde_json::to_string(&payload)?)?;
    } else {
        write_stdout_line(&format!("providers={}", rows.len()))?;
        for row in rows {
            write_stdout_line(&format!(
                "provider={}\tname={}\tmethods={}",
                row.id,
                row.name,
                methods_as_pipe(&row.methods)
            ))?;
        }
    }

    Ok(())
}

pub fn auth_login_priority(provider_id: &str) -> usize {
    match provider_id {
        "opencode" => 0,
        "anthropic" => 1,
        "github-copilot" => 2,
        "openai" => 3,
        "google" => 4,
        "openrouter" => 5,
        "vercel" => 6,
        _ => 100,
    }
}

pub fn load_models_index() -> Result<BTreeMap<String, ModelsProvider>> {
    let path = resolve_models_path().context("failed to resolve models index path")?;
    let content = std::fs::read_to_string(&path)
        .context(format!("failed to read models index at {}", path.display()))?;
    serde_json::from_str(&content).context("failed to parse models index")
}

pub fn resolve_models_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUSTCODE_MODELS_PATH") {
        return Some(PathBuf::from(path));
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let path = PathBuf::from(home).join(".cache/opencode/models.json");
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

pub fn methods_as_csv(methods: &[AuthMethod]) -> String {
    methods
        .iter()
        .map(|method| method.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_auth_methods(methods: &[String]) -> Vec<AuthMethod> {
    methods
        .iter()
        .filter_map(|method| match method.as_str() {
            "api_key" => Some(AuthMethod::ApiKey),
            "oauth_device_code" => Some(AuthMethod::OAuthDeviceCode),
            "oauth_browser" => Some(AuthMethod::OAuthBrowser),
            _ => None,
        })
        .collect()
}

fn fallback_provider_name(id: &str) -> &'static str {
    match id {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "github-copilot" => "GitHub Copilot",
        "gitlab" => "GitLab",
        _ => "Provider",
    }
}

fn methods_as_pipe(methods: &[AuthMethod]) -> String {
    methods
        .iter()
        .map(|method| method.as_str())
        .collect::<Vec<_>>()
        .join("|")
}
