use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::sleep;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoredCredential {
    ApiKey {
        key: String,
    },
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        expires_at_unix: Option<i64>,
        account_id: Option<String>,
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
    pub fn open_default() -> Self {
        Self {
            path: default_auth_path(),
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, provider: &str) -> Result<Option<StoredCredential>, AuthError> {
        validate_provider(provider)?;
        let auth = self.read_file()?;
        Ok(auth.providers.get(provider).cloned())
    }

    pub fn get_api_key(&self, provider: &str) -> Result<Option<String>, AuthError> {
        let Some(credential) = self.get(provider)? else {
            return Ok(None);
        };
        match credential {
            StoredCredential::ApiKey { key } => Ok(Some(key)),
            StoredCredential::OAuth { .. } => Ok(None),
        }
    }

    pub fn set_api_key(&self, provider: &str, key: &str) -> Result<(), AuthError> {
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
            },
        );
        self.write_file(&auth)
    }

    pub fn remove(&self, provider: &str) -> Result<bool, AuthError> {
        validate_provider(provider)?;
        let mut auth = self.read_file()?;
        let removed = auth.providers.remove(provider).is_some();
        self.write_file(&auth)?;
        Ok(removed)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    ApiKey,
    OAuthDeviceCode,
}

impl AuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::OAuthDeviceCode => "oauth_device_code",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthLoginHint {
    pub authorize_url: String,
    pub instructions: String,
}

pub fn methods_for_provider(provider_id: &str) -> Vec<AuthMethod> {
    let provider = provider_id.to_ascii_lowercase();
    if OAUTH_PROVIDERS.contains(&provider.as_str()) {
        vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
    } else {
        vec![AuthMethod::ApiKey]
    }
}

pub fn oauth_login_hint(provider_id: &str) -> Option<OAuthLoginHint> {
    let provider = provider_id.to_ascii_lowercase();
    let (url, instructions) = match provider.as_str() {
        "openai" => (
            "https://chatgpt.com",
            "Complete ChatGPT authorization flow, then store the resulting token with `rustcode auth login openai --from-env <ENV_VAR>`.",
        ),
        "github-copilot" | "github-copilot-enterprise" => (
            "https://github.com/login/device",
            "Complete GitHub device login, then store the resulting token with `rustcode auth login <provider> --from-env <ENV_VAR>`.",
        ),
        "gitlab" => (
            "https://gitlab.com/oauth/authorize",
            "Complete GitLab OAuth login in browser, then store the resulting token with `rustcode auth login <provider> --from-env <ENV_VAR>`.",
        ),
        _ => return None,
    };
    Some(OAuthLoginHint {
        authorize_url: url.to_string(),
        instructions: instructions.to_string(),
    })
}

pub fn known_oauth_providers() -> &'static [&'static str] {
    OAUTH_PROVIDERS
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeFlowStart {
    pub provider: String,
    pub domain: String,
    pub verification_uri: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

pub async fn start_device_code_flow(
    provider: &str,
    domain: Option<&str>,
) -> Result<DeviceCodeFlowStart, AuthError> {
    match provider {
        "github-copilot" | "github-copilot-enterprise" => {
            let normalized_domain = normalize_domain(domain.unwrap_or("github.com"))?;
            start_github_device_code(provider, &normalized_domain).await
        }
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support device code flow"
        ))),
    }
}

pub async fn poll_device_code_flow_for_api_key(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<String, AuthError> {
    match flow.provider.as_str() {
        "github-copilot" | "github-copilot-enterprise" => {
            poll_github_device_code(flow, timeout).await
        }
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support device code polling"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct GithubDeviceCodeResponse {
    verification_uri: String,
    user_code: String,
    device_code: String,
    interval: Option<u64>,
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GithubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    interval: Option<u64>,
}

const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

async fn start_github_device_code(
    provider: &str,
    domain: &str,
) -> Result<DeviceCodeFlowStart, AuthError> {
    let url = format!("https://{domain}/login/device/code");
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": GITHUB_CLIENT_ID,
            "scope": "read:user"
        }))
        .send()
        .await
        .map_err(|err| AuthError::Network(err.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| AuthError::Network(err.to_string()))?;
    if !status.is_success() {
        return Err(AuthError::OAuthFailed(format!(
            "device code request failed with {status}: {body}"
        )));
    }

    let parsed: GithubDeviceCodeResponse =
        serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;

    Ok(DeviceCodeFlowStart {
        provider: provider.to_string(),
        domain: domain.to_string(),
        verification_uri: parsed.verification_uri,
        user_code: parsed.user_code,
        device_code: parsed.device_code,
        interval_secs: parsed.interval.unwrap_or(5).max(1),
        expires_in_secs: parsed.expires_in.unwrap_or(900),
    })
}

async fn poll_github_device_code(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<String, AuthError> {
    let client = reqwest::Client::new();
    let token_url = format!("https://{}/login/oauth/access_token", flow.domain);
    let deadline = Instant::now() + timeout;
    let mut interval = flow.interval_secs.max(1);

    while Instant::now() < deadline {
        let response = client
            .post(&token_url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": GITHUB_CLIENT_ID,
                "device_code": flow.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
            }))
            .send()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;
        if !status.is_success() {
            return Err(AuthError::OAuthFailed(format!(
                "oauth polling failed with {status}: {body}"
            )));
        }

        let parsed: GithubTokenResponse =
            serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;
        if let Some(token) = parsed.access_token {
            return Ok(token);
        }

        let error = parsed
            .error
            .unwrap_or_else(|| "unknown_error".to_string())
            .to_ascii_lowercase();
        match error.as_str() {
            "authorization_pending" => {}
            "slow_down" => {
                interval = interval.saturating_add(1);
            }
            "expired_token" => return Err(AuthError::OAuthTimeout),
            _ => {
                return Err(AuthError::OAuthFailed(
                    parsed
                        .error_description
                        .unwrap_or_else(|| "device flow failed".to_string()),
                ))
            }
        }

        if let Some(server_interval) = parsed.interval {
            interval = interval.max(server_interval);
        }
        sleep(Duration::from_secs(interval)).await;
    }

    Err(AuthError::OAuthTimeout)
}

fn normalize_domain(raw: &str) -> Result<String, AuthError> {
    if raw.trim().is_empty() {
        return Err(AuthError::Validation(
            "domain must not be empty".to_string(),
        ));
    }
    let without_scheme = raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    Ok(without_scheme.trim_end_matches('/').to_string())
}

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
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn api_key_round_trip_and_remove() {
        let path = make_temp_file_path("auth-roundtrip");
        let store = AuthStore::with_path(path);

        store
            .set_api_key("openrouter", "secret-123")
            .expect("set key should succeed");
        assert_eq!(
            store
                .get_api_key("openrouter")
                .expect("get key should succeed")
                .as_deref(),
            Some("secret-123")
        );

        let removed = store.remove("openrouter").expect("remove should succeed");
        assert!(removed);
        assert!(store
            .get_api_key("openrouter")
            .expect("get should succeed")
            .is_none());
    }

    #[test]
    fn oauth_method_mapping_is_provider_specific() {
        assert_eq!(
            methods_for_provider("openai"),
            vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
        );
        assert_eq!(
            methods_for_provider("github-copilot"),
            vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
        );
        assert_eq!(methods_for_provider("openrouter"), vec![AuthMethod::ApiKey]);
    }

    #[test]
    fn oauth_hint_exists_for_supported_provider() {
        let hint = oauth_login_hint("gitlab").expect("hint must exist");
        assert!(hint.authorize_url.contains("gitlab"));
    }

    #[test]
    fn normalizes_domain_for_device_flow() {
        assert_eq!(
            normalize_domain("https://github.com/").expect("must normalize"),
            "github.com"
        );
        assert_eq!(
            normalize_domain("company.ghe.com").expect("must normalize"),
            "company.ghe.com"
        );
    }

    fn make_temp_file_path(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("monotonic time")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("rustcode-auth-{name}-{pid}-{now}.json"))
    }
}
