use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(test)]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(test)]
use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MCP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_DISCOVERY_HEADER: &str = "MCP-Protocol-Version";
const MCP_DISCOVERY_VERSION: &str = "2024-11-05";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("failed to read auth store: {0}")]
    Read(String),
    #[error("failed to write auth store: {0}")]
    Write(String),
    #[error("failed to parse auth store: {0}")]
    Parse(String),
    #[error("failed to validate input: {0}")]
    Validation(String),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("oauth authorization timeout")]
    OAuthTimeout,
    #[error("oauth authorization failed: {0}")]
    OAuthFailed(String),
}

pub(crate) fn auth_http_client() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .timeout(AUTH_HTTP_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|err| AuthError::Network(format!("failed to build http client: {err}")))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredCredential {
    ApiKey {
        key: String,
        #[serde(default)]
        domain: Option<String>,
    },
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        expires_at_unix: Option<i64>,
        account_id: Option<String>,
        #[serde(default)]
        domain: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    providers: BTreeMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
pub struct AuthStore {
    path: PathBuf,
}

impl AuthStore {
    #[must_use]
    pub fn open_default() -> Self {
        Self {
            path: default_auth_path(),
        }
    }

    #[must_use]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the stored credential for `provider`.
    ///
    /// # Errors
    /// Returns `AuthError` if `provider` is invalid, or if the auth store cannot be read/parsed.
    pub fn get(&self, provider: &str) -> Result<Option<StoredCredential>, AuthError> {
        validate_provider(provider)?;
        let auth = self.read_file()?;
        Ok(auth.providers.get(provider).cloned())
    }

    /// Load the stored API key for `provider`, if present.
    ///
    /// # Errors
    /// Returns `AuthError` if `provider` is invalid, or if the auth store cannot be read/parsed.
    pub fn get_api_key(&self, provider: &str) -> Result<Option<String>, AuthError> {
        let Some(credential) = self.get(provider)? else {
            return Ok(None);
        };
        match credential {
            StoredCredential::ApiKey { key, .. } => Ok(Some(key)),
            StoredCredential::OAuth { .. } => Ok(None),
        }
    }

    /// Store an API key credential for `provider`.
    ///
    /// # Errors
    /// Returns `AuthError` if input validation fails or if the auth store cannot be written.
    pub fn set_api_key(&self, provider: &str, key: &str) -> Result<(), AuthError> {
        self.set_api_key_with_domain(provider, key, None)
    }

    /// Store an API key credential for `provider` with an optional domain override.
    ///
    /// # Errors
    /// Returns `AuthError` if input validation fails or if the auth store cannot be written.
    pub fn set_api_key_with_domain(
        &self,
        provider: &str,
        key: &str,
        domain: Option<&str>,
    ) -> Result<(), AuthError> {
        validate_provider(provider)?;
        if key.trim().is_empty() {
            return Err(AuthError::Validation(
                "api key must not be empty".to_string(),
            ));
        }

        let mut auth = self.read_file()?;
        auth.providers.insert(
            provider.to_string(),
            StoredCredential::ApiKey {
                key: key.to_string(),
                domain: domain
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            },
        );
        self.write_file(&auth)
    }

    /// Store an OAuth credential for `provider`.
    ///
    /// # Errors
    /// Returns `AuthError` if input validation fails or if the auth store cannot be written.
    pub fn set_oauth(
        &self,
        provider: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at_unix: Option<i64>,
        account_id: Option<&str>,
    ) -> Result<(), AuthError> {
        validate_provider(provider)?;
        if access_token.trim().is_empty() {
            return Err(AuthError::Validation(
                "oauth access token must not be empty".to_string(),
            ));
        }

        let mut auth = self.read_file()?;
        auth.providers.insert(
            provider.to_string(),
            StoredCredential::OAuth {
                access_token: access_token.to_string(),
                refresh_token: refresh_token.map(ToOwned::to_owned),
                expires_at_unix,
                account_id: account_id.map(ToOwned::to_owned),
                domain: None,
            },
        );
        self.write_file(&auth)
    }

    /// Remove the stored credential for `provider` (if any).
    ///
    /// # Errors
    /// Returns `AuthError` if `provider` is invalid, or if the auth store cannot be read/written.
    pub fn remove(&self, provider: &str) -> Result<bool, AuthError> {
        validate_provider(provider)?;
        let mut auth = self.read_file()?;
        let removed = auth.providers.remove(provider).is_some();
        self.write_file(&auth)?;
        Ok(removed)
    }

    /// List provider ids currently present in the auth store.
    ///
    /// # Errors
    /// Returns `AuthError` if the auth store cannot be read/parsed.
    pub fn providers(&self) -> Result<Vec<String>, AuthError> {
        let mut providers: Vec<String> = self.read_file()?.providers.keys().cloned().collect();
        providers.sort();
        Ok(providers)
    }

    fn read_file(&self) -> Result<AuthFile, AuthError> {
        if !self.path.exists() {
            return Ok(AuthFile::default());
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|err| AuthError::Read(format!("{}: {err}", self.path.display())))?;
        if raw.trim().is_empty() {
            return Ok(AuthFile::default());
        }
        serde_json::from_str::<AuthFile>(&raw)
            .map_err(|err| AuthError::Parse(format!("{}: {err}", self.path.display())))
    }

    fn write_file(&self, auth: &AuthFile) -> Result<(), AuthError> {
        let parent = self.path.parent().ok_or_else(|| {
            AuthError::Write(format!(
                "auth path has no parent directory: {}",
                self.path.display()
            ))
        })?;
        fs::create_dir_all(parent)
            .map_err(|err| AuthError::Write(format!("{}: {err}", parent.display())))?;

        let serialized = serde_json::to_string_pretty(auth)
            .map_err(|err| AuthError::Write(format!("serialize auth store: {err}")))?;
        let temp_path = self.path.with_extension("json.tmp");
        fs::write(&temp_path, serialized)
            .map_err(|err| AuthError::Write(format!("{}: {err}", temp_path.display())))?;
        set_owner_read_write_only(&temp_path)
            .map_err(|err| AuthError::Write(format!("{}: {err}", temp_path.display())))?;
        fs::rename(&temp_path, &self.path).map_err(|err| {
            AuthError::Write(format!(
                "rename {} -> {}: {err}",
                temp_path.display(),
                self.path.display()
            ))
        })?;
        set_owner_read_write_only(&self.path)
            .map_err(|err| AuthError::Write(format!("{}: {err}", self.path.display())))?;
        Ok(())
    }
}

impl StoredCredential {
    /// Check if this credential has expired.
    ///
    /// Only meaningful for OAuth credentials with an `expires_at_unix` field.
    /// API key credentials never expire. Uses a 60-second safety margin to
    /// avoid using tokens that are about to expire.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        match self {
            Self::ApiKey { .. } => false,
            Self::OAuth {
                expires_at_unix: Some(expires_at),
                ..
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                // 60-second safety margin
                *expires_at <= now + 60
            }
            Self::OAuth {
                expires_at_unix: None,
                ..
            } => false, // No expiry set — assume valid
        }
    }

    /// Extract the usable access token from this credential.
    #[must_use]
    pub fn access_token(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key, .. } => Some(key),
            Self::OAuth { access_token, .. } => Some(access_token),
        }
    }

    /// Extract the refresh token, if present.
    #[must_use]
    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            Self::ApiKey { .. } => None,
            Self::OAuth { refresh_token, .. } => refresh_token.as_deref(),
        }
    }
}

impl AuthStore {
    /// Get the access token for a provider, returning `None` if the token
    /// has expired (caller should trigger a refresh flow).
    ///
    /// # Errors
    /// Returns `AuthError` if the auth store cannot be read.
    pub fn get_active_token(&self, provider: &str) -> Result<Option<String>, AuthError> {
        let Some(cred) = self.get(provider)? else {
            return Ok(None);
        };
        if cred.is_expired() {
            return Ok(None);
        }
        Ok(cred.access_token().map(ToOwned::to_owned))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    ApiKey,
    OAuthDeviceCode,
    OAuthBrowser,
}

impl AuthMethod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::OAuthDeviceCode => "oauth_device_code",
            Self::OAuthBrowser => "oauth_browser",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthLoginHint {
    pub authorize_url: String,
    pub instructions: String,
}

#[must_use]
pub fn methods_for_provider(provider_id: &str) -> Vec<AuthMethod> {
    let provider = provider_id.to_ascii_lowercase();
    match provider.as_str() {
        "openai" => vec![
            AuthMethod::OAuthDeviceCode,
            AuthMethod::OAuthBrowser,
            AuthMethod::ApiKey,
        ],
        "github-copilot" | "github-copilot-enterprise" => {
            vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
        }
        "gitlab" => vec![AuthMethod::OAuthBrowser, AuthMethod::ApiKey],
        _ => vec![AuthMethod::ApiKey],
    }
}

#[must_use]
pub fn oauth_login_hint(provider_id: &str) -> Option<OAuthLoginHint> {
    let provider = provider_id.to_ascii_lowercase();
    let (url, instructions) = match provider.as_str() {
        "openai" => (
            "https://chatgpt.com",
            "Use `rustcode auth login openai` for headless device flow, or `rustcode auth login openai --method oauth_browser` for browser OAuth.",
        ),
        "github-copilot" | "github-copilot-enterprise" => (
            "https://github.com/login/device",
            "Complete GitHub device login, then store the resulting token with `rustcode auth login <provider> --from-env <ENV_VAR>`.",
        ),
        "gitlab" => (
            "https://gitlab.com/oauth/authorize",
            "Use `rustcode auth login gitlab --method oauth_browser` for browser OAuth, or `rustcode auth login gitlab --from-env <ENV_VAR>` for personal access tokens.",
        ),
        _ => return None,
    };
    Some(OAuthLoginHint {
        authorize_url: url.to_string(),
        instructions: instructions.to_string(),
    })
}

#[must_use]
pub fn known_oauth_providers() -> &'static [&'static str] {
    OAUTH_PROVIDERS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthDiscovery {
    pub supported: bool,
    pub metadata_url: Option<String>,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpOAuthDiscoveryMetadata {
    #[serde(default)]
    authorization_endpoint: Option<String>,
    #[serde(default)]
    token_endpoint: Option<String>,
}

/// Discover MCP OAuth metadata endpoints for a server URL.
///
/// # Errors
/// Returns `AuthError` if the URL is invalid, the server is unreachable, or metadata parsing fails.
pub async fn discover_mcp_oauth(url: &str) -> Result<McpOAuthDiscovery, AuthError> {
    let base_url = reqwest::Url::parse(url)
        .map_err(|err| AuthError::Validation(format!("invalid MCP URL `{url}`: {err}")))?;
    let client = reqwest::Client::builder()
        .timeout(MCP_DISCOVERY_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|err| AuthError::Network(format!("failed to create discovery client: {err}")))?;

    let mut last_error: Option<String> = None;
    for path in mcp_discovery_paths(base_url.path()) {
        let mut candidate = base_url.clone();
        candidate.set_path(&path);

        let response = match client
            .get(candidate.clone())
            .header(MCP_DISCOVERY_HEADER, MCP_DISCOVERY_VERSION)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                last_error = Some(err.to_string());
                continue;
            }
        };

        if response.status() != reqwest::StatusCode::OK {
            continue;
        }

        let metadata = match response.json::<McpOAuthDiscoveryMetadata>().await {
            Ok(metadata) => metadata,
            Err(err) => {
                last_error = Some(err.to_string());
                continue;
            }
        };

        if metadata.authorization_endpoint.is_some() && metadata.token_endpoint.is_some() {
            return Ok(McpOAuthDiscovery {
                supported: true,
                metadata_url: Some(candidate.to_string()),
                authorization_endpoint: metadata.authorization_endpoint,
                token_endpoint: metadata.token_endpoint,
            });
        }
    }

    let _ = last_error;
    Ok(McpOAuthDiscovery {
        supported: false,
        metadata_url: None,
        authorization_endpoint: None,
        token_endpoint: None,
    })
}

fn mcp_discovery_paths(base_path: &str) -> Vec<String> {
    let trimmed = base_path.trim_start_matches('/').trim_end_matches('/');
    let canonical = "/.well-known/oauth-authorization-server".to_string();

    if trimmed.is_empty() {
        return vec![canonical];
    }

    let mut candidates = Vec::new();
    let mut push_unique = |candidate: String| {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    push_unique(format!("{canonical}/{trimmed}"));
    push_unique(format!("/{trimmed}/.well-known/oauth-authorization-server"));
    push_unique(canonical);
    candidates
}

mod oauth;

pub use oauth::{
    complete_browser_oauth_flow, complete_mcp_browser_oauth_flow, normalize_domain,
    poll_device_code_flow_for_api_key, poll_device_code_flow_for_credential,
    start_browser_oauth_flow, start_device_code_flow, start_mcp_browser_oauth_flow,
    BrowserOAuthFlowStart, DeviceCodeFlowCredential, DeviceCodeFlowStart,
    McpBrowserOAuthFlowStart,
};

#[cfg(test)]
pub(crate) use oauth::{
    deserialize_u64_string_or_number, extract_openai_account_id_from_jwt,
    GITLAB_BUNDLED_CLIENT_ID,
};

fn validate_provider(provider: &str) -> Result<(), AuthError> {
    if provider.trim().is_empty() {
        return Err(AuthError::Validation(
            "provider id must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn default_auth_path() -> PathBuf {
    if let Ok(path) = std::env::var("RUSTCODE_AUTH_FILE") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("rustcode/auth.json");
    }
    if let Ok(path) = std::env::var("HOME") {
        return PathBuf::from(path).join(".local/share/rustcode/auth.json");
    }
    PathBuf::from(".rustcode-auth.json")
}

#[cfg(unix)]
fn set_owner_read_write_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_owner_read_write_only(path: &Path) -> std::io::Result<()> {
    let _ = path;
    Ok(())
}

const OAUTH_PROVIDERS: &[&str] = &[
    "openai",
    "github-copilot",
    "github-copilot-enterprise",
    "gitlab",
];

#[cfg(test)]
mod tests;
