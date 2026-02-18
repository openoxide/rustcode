use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::Digest;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::sleep;

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

fn auth_http_client() -> Result<reqwest::Client, AuthError> {
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
            StoredCredential::ApiKey { key, .. } => Ok(Some(key)),
            StoredCredential::OAuth { .. } => Ok(None),
        }
    }

    pub fn set_api_key(&self, provider: &str, key: &str) -> Result<(), AuthError> {
        self.set_api_key_with_domain(provider, key, None)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    ApiKey,
    OAuthDeviceCode,
    OAuthBrowser,
}

impl AuthMethod {
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
        "openai" => start_openai_device_code(provider).await,
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
    let credential = poll_device_code_flow_for_credential(flow, timeout).await?;
    Ok(credential.access_token)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeFlowCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<u64>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserOAuthFlowStart {
    pub provider: String,
    pub domain: String,
    pub authorize_url: String,
    pub redirect_uri: String,
    client_id: String,
    state: String,
    code_verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpBrowserOAuthFlowStart {
    pub server_name: String,
    pub server_url: String,
    pub authorize_url: String,
    pub redirect_uri: String,
    pub metadata_url: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub scopes: Vec<String>,
    client_id: String,
    state: String,
    code_verifier: String,
}

pub async fn start_browser_oauth_flow(
    provider: &str,
    domain: Option<&str>,
    client_id: Option<&str>,
    callback_port: u16,
) -> Result<BrowserOAuthFlowStart, AuthError> {
    match provider {
        "openai" => start_openai_browser_oauth_flow(domain, callback_port),
        "gitlab" => start_gitlab_browser_oauth_flow(domain, client_id, callback_port),
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support browser oauth flow"
        ))),
    }
}

pub async fn complete_browser_oauth_flow(
    flow: &BrowserOAuthFlowStart,
    timeout: Duration,
    client_secret: Option<&str>,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    match flow.provider.as_str() {
        "openai" => complete_openai_browser_oauth_flow(flow, timeout).await,
        "gitlab" => complete_gitlab_browser_oauth_flow(flow, timeout, client_secret).await,
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support browser oauth completion"
        ))),
    }
}

pub fn start_mcp_browser_oauth_flow(
    server_name: &str,
    server_url: &str,
    discovery: &McpOAuthDiscovery,
    client_id: &str,
    callback_port: u16,
    scopes: &[String],
) -> Result<McpBrowserOAuthFlowStart, AuthError> {
    if callback_port == 0 {
        return Err(AuthError::Validation(
            "oauth callback port must be between 1 and 65535".to_string(),
        ));
    }
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err(AuthError::Validation(
            "mcp oauth browser flow requires a non-empty client_id".to_string(),
        ));
    }
    if !discovery.supported {
        return Err(AuthError::Validation(
            "mcp oauth discovery did not include authorization/token endpoints".to_string(),
        ));
    }
    let authorization_endpoint = discovery.authorization_endpoint.clone().ok_or_else(|| {
        AuthError::Validation("mcp oauth metadata is missing authorization_endpoint".to_string())
    })?;
    let token_endpoint = discovery.token_endpoint.clone().ok_or_else(|| {
        AuthError::Validation("mcp oauth metadata is missing token_endpoint".to_string())
    })?;

    let redirect_uri = format!("http://127.0.0.1:{callback_port}{MCP_OAUTH_CALLBACK_PATH}");
    let state = generate_oauth_state();
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_pkce_code_challenge(&code_verifier);

    let mut authorize_url = reqwest::Url::parse(&authorization_endpoint).map_err(|err| {
        AuthError::Validation(format!("invalid mcp oauth authorize endpoint: {err}"))
    })?;
    {
        let mut query = authorize_url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", client_id);
        query.append_pair("redirect_uri", &redirect_uri);
        if !scopes.is_empty() {
            query.append_pair("scope", &scopes.join(" "));
        }
        query.append_pair("state", &state);
        query.append_pair("code_challenge", &code_challenge);
        query.append_pair("code_challenge_method", "S256");
    }

    Ok(McpBrowserOAuthFlowStart {
        server_name: server_name.to_string(),
        server_url: server_url.to_string(),
        authorize_url: authorize_url.to_string(),
        redirect_uri,
        metadata_url: discovery.metadata_url.clone(),
        authorization_endpoint,
        token_endpoint,
        scopes: scopes.to_vec(),
        client_id: client_id.to_string(),
        state,
        code_verifier,
    })
}

pub async fn complete_mcp_browser_oauth_flow(
    flow: &McpBrowserOAuthFlowStart,
    timeout: Duration,
    client_secret: Option<&str>,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let callback = callback_route_from_redirect_uri(&flow.redirect_uri)?;
    let code = wait_for_oauth_callback(&callback, &flow.state, timeout).await?;

    let client = auth_http_client()?;
    let mut form_params = BTreeMap::new();
    form_params.insert("client_id".to_string(), flow.client_id.clone());
    form_params.insert("code".to_string(), code);
    form_params.insert("grant_type".to_string(), "authorization_code".to_string());
    form_params.insert("redirect_uri".to_string(), flow.redirect_uri.clone());
    form_params.insert("code_verifier".to_string(), flow.code_verifier.clone());
    if let Some(secret) = client_secret.filter(|value| !value.trim().is_empty()) {
        form_params.insert("client_secret".to_string(), secret.to_string());
    }

    let response = client
        .post(&flow.token_endpoint)
        .header("Accept", "application/json")
        .form(&form_params)
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
            "mcp token exchange failed with {status}: {body}"
        )));
    }

    let token: McpTokenExchangeResponse =
        serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;
    if let Some(error) = token.error.as_deref() {
        let detail = token
            .error_description
            .as_deref()
            .unwrap_or("mcp oauth exchange failed");
        return Err(AuthError::OAuthFailed(format!("{error}: {detail}")));
    }
    let access_token = token.access_token.ok_or_else(|| {
        AuthError::Parse("missing access_token in mcp oauth token response".to_string())
    })?;

    Ok(DeviceCodeFlowCredential {
        access_token,
        refresh_token: token.refresh_token,
        expires_in_secs: token.expires_in,
        account_id: None,
    })
}

pub async fn poll_device_code_flow_for_credential(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    match flow.provider.as_str() {
        "openai" => poll_openai_device_code(flow, timeout).await,
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

#[derive(Debug, Deserialize)]
struct OpenAiDeviceCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default, deserialize_with = "deserialize_u64_string_or_number")]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct OpenAiDeviceTokenPollResponse {
    authorization_code: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiTokenExchangeResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitlabTokenExchangeResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpTokenExchangeResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

const OPENAI_ISSUER: &str = "https://auth.openai.com";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const GITLAB_BUNDLED_CLIENT_ID: &str =
    "1d89f9fdb23ee96d4e603201f6861dab6e143c5c3c00469a018a2d94bdc03d4e";
const GITLAB_DEFAULT_SCOPES: &str = "api read_user read_repository";
const OPENAI_BROWSER_SCOPES: &str = "openid profile email offline_access";
const GITLAB_OAUTH_CALLBACK_PATH: &str = "/callback";
const MCP_OAUTH_CALLBACK_PATH: &str = "/auth/callback";
const OPENAI_OAUTH_CALLBACK_PATH: &str = "/auth/callback";

async fn start_github_device_code(
    provider: &str,
    domain: &str,
) -> Result<DeviceCodeFlowStart, AuthError> {
    let url = format!("https://{domain}/login/device/code");
    let client = auth_http_client()?;
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

async fn start_openai_device_code(provider: &str) -> Result<DeviceCodeFlowStart, AuthError> {
    let client = auth_http_client()?;
    let response = client
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/usercode"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": OPENAI_CLIENT_ID
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

    let parsed: OpenAiDeviceCodeResponse =
        serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;

    Ok(DeviceCodeFlowStart {
        provider: provider.to_string(),
        domain: "auth.openai.com".to_string(),
        verification_uri: format!("{OPENAI_ISSUER}/codex/device"),
        user_code: parsed.user_code,
        device_code: parsed.device_auth_id,
        interval_secs: parsed.interval.max(1),
        expires_in_secs: 900,
    })
}

async fn poll_github_device_code(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let client = auth_http_client()?;
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
            return Ok(DeviceCodeFlowCredential {
                access_token: token,
                refresh_token: None,
                expires_in_secs: None,
                account_id: None,
            });
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

async fn poll_openai_device_code(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let client = auth_http_client()?;
    let deadline = Instant::now() + timeout;
    let interval = flow.interval_secs.max(1);

    while Instant::now() < deadline {
        let response = client
            .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/token"))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "device_auth_id": flow.device_code,
                "user_code": flow.user_code,
            }))
            .send()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;

        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
            sleep(Duration::from_secs(interval)).await;
            continue;
        }
        if !status.is_success() {
            return Err(AuthError::OAuthFailed(format!(
                "oauth polling failed with {status}: {body}"
            )));
        }

        let polled: OpenAiDeviceTokenPollResponse =
            serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;
        let authorization_code = polled.authorization_code.ok_or_else(|| {
            AuthError::Parse(
                "missing authorization_code in openai device token response".to_string(),
            )
        })?;
        let code_verifier = polled.code_verifier.ok_or_else(|| {
            AuthError::Parse("missing code_verifier in openai device token response".to_string())
        })?;

        let exchange = client
            .post(format!("{OPENAI_ISSUER}/oauth/token"))
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(
                reqwest::Url::parse_with_params(
                    "http://localhost",
                    &[
                        ("grant_type", "authorization_code"),
                        ("code", authorization_code.as_str()),
                        (
                            "redirect_uri",
                            "https://auth.openai.com/deviceauth/callback",
                        ),
                        ("client_id", OPENAI_CLIENT_ID),
                        ("code_verifier", code_verifier.as_str()),
                    ],
                )
                .map_err(|err| AuthError::Parse(err.to_string()))?
                .query()
                .unwrap_or_default()
                .to_string(),
            )
            .send()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;

        let exchange_status = exchange.status();
        let exchange_body = exchange
            .text()
            .await
            .map_err(|err| AuthError::Network(err.to_string()))?;
        if !exchange_status.is_success() {
            return Err(AuthError::OAuthFailed(format!(
                "openai token exchange failed with {exchange_status}: {exchange_body}"
            )));
        }
        let token: OpenAiTokenExchangeResponse = serde_json::from_str(&exchange_body)
            .map_err(|err| AuthError::Parse(err.to_string()))?;
        return Ok(DeviceCodeFlowCredential {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_in_secs: token.expires_in,
            account_id: None,
        });
    }

    Err(AuthError::OAuthTimeout)
}

fn start_openai_browser_oauth_flow(
    domain: Option<&str>,
    callback_port: u16,
) -> Result<BrowserOAuthFlowStart, AuthError> {
    if callback_port == 0 {
        return Err(AuthError::Validation(
            "oauth callback port must be between 1 and 65535".to_string(),
        ));
    }

    let normalized_domain = normalize_domain(domain.unwrap_or("auth.openai.com"))?;
    let issuer = format!("https://{normalized_domain}");
    let redirect_uri = format!("http://127.0.0.1:{callback_port}{OPENAI_OAUTH_CALLBACK_PATH}");
    let state = generate_oauth_state();
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_pkce_code_challenge(&code_verifier);

    let mut authorize_url =
        reqwest::Url::parse(&format!("{issuer}/oauth/authorize")).map_err(|err| {
            AuthError::Validation(format!("invalid openai oauth authorize url: {err}"))
        })?;
    {
        let mut query = authorize_url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", OPENAI_CLIENT_ID);
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("scope", OPENAI_BROWSER_SCOPES);
        query.append_pair("code_challenge", &code_challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", &state);
        query.append_pair("id_token_add_organizations", "true");
        query.append_pair("codex_cli_simplified_flow", "true");
        query.append_pair("originator", "rustcode");
    }

    Ok(BrowserOAuthFlowStart {
        provider: "openai".to_string(),
        domain: normalized_domain,
        authorize_url: authorize_url.to_string(),
        redirect_uri,
        client_id: OPENAI_CLIENT_ID.to_string(),
        state,
        code_verifier,
    })
}

fn start_gitlab_browser_oauth_flow(
    domain: Option<&str>,
    client_id: Option<&str>,
    callback_port: u16,
) -> Result<BrowserOAuthFlowStart, AuthError> {
    if callback_port == 0 {
        return Err(AuthError::Validation(
            "oauth callback port must be between 1 and 65535".to_string(),
        ));
    }

    let normalized_domain = normalize_domain(domain.unwrap_or("gitlab.com"))?;
    let is_gitlab_dot_com = normalized_domain.eq_ignore_ascii_case("gitlab.com");
    let client_id = client_id.map(str::trim).filter(|value| !value.is_empty());
    let resolved_client_id = match (client_id, is_gitlab_dot_com) {
        (Some(value), _) => value.to_string(),
        (None, true) => GITLAB_BUNDLED_CLIENT_ID.to_string(),
        (None, false) => {
            return Err(AuthError::Validation(
                "gitlab browser oauth for self-hosted domains requires GITLAB_OAUTH_CLIENT_ID"
                    .to_string(),
            ))
        }
    };
    let redirect_uri = format!("http://127.0.0.1:{callback_port}{GITLAB_OAUTH_CALLBACK_PATH}");
    let state = generate_oauth_state();
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_pkce_code_challenge(&code_verifier);

    let mut authorize_url = reqwest::Url::parse(&format!(
        "https://{normalized_domain}/oauth/authorize"
    ))
    .map_err(|err| AuthError::Validation(format!("invalid gitlab oauth authorize url: {err}")))?;

    {
        let mut query = authorize_url.query_pairs_mut();
        query.append_pair("client_id", &resolved_client_id);
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("response_type", "code");
        query.append_pair("scope", GITLAB_DEFAULT_SCOPES);
        query.append_pair("state", &state);
        query.append_pair("code_challenge", &code_challenge);
        query.append_pair("code_challenge_method", "S256");
    }

    Ok(BrowserOAuthFlowStart {
        provider: "gitlab".to_string(),
        domain: normalized_domain,
        authorize_url: authorize_url.to_string(),
        redirect_uri,
        client_id: resolved_client_id,
        state,
        code_verifier,
    })
}

async fn complete_gitlab_browser_oauth_flow(
    flow: &BrowserOAuthFlowStart,
    timeout: Duration,
    client_secret: Option<&str>,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let callback = callback_route_from_redirect_uri(&flow.redirect_uri)?;
    let code = wait_for_oauth_callback(&callback, &flow.state, timeout).await?;

    let scheme = if flow.domain.starts_with("127.0.0.1:") || flow.domain.starts_with("localhost:") {
        "http"
    } else {
        "https"
    };
    let token_url = format!("{scheme}://{}/oauth/token", flow.domain);
    let client = auth_http_client()?;
    let mut form_params = BTreeMap::new();
    form_params.insert("client_id".to_string(), flow.client_id.clone());
    form_params.insert("code".to_string(), code);
    form_params.insert("grant_type".to_string(), "authorization_code".to_string());
    form_params.insert("redirect_uri".to_string(), flow.redirect_uri.clone());
    form_params.insert("code_verifier".to_string(), flow.code_verifier.clone());
    if let Some(secret) = client_secret.filter(|value| !value.trim().is_empty()) {
        form_params.insert("client_secret".to_string(), secret.to_string());
    }

    let response = client
        .post(token_url)
        .header("Accept", "application/json")
        .form(&form_params)
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
            "gitlab token exchange failed with {status}: {body}"
        )));
    }

    let token: GitlabTokenExchangeResponse =
        serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;
    if let Some(error) = token.error.as_deref() {
        let detail = token
            .error_description
            .as_deref()
            .unwrap_or("gitlab oauth exchange failed");
        return Err(AuthError::OAuthFailed(format!("{error}: {detail}")));
    }
    let access_token = token.access_token.ok_or_else(|| {
        AuthError::Parse("missing access_token in gitlab oauth token response".to_string())
    })?;

    Ok(DeviceCodeFlowCredential {
        access_token,
        refresh_token: token.refresh_token,
        expires_in_secs: token.expires_in,
        account_id: None,
    })
}

async fn complete_openai_browser_oauth_flow(
    flow: &BrowserOAuthFlowStart,
    timeout: Duration,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let callback = callback_route_from_redirect_uri(&flow.redirect_uri)?;
    let code = wait_for_oauth_callback(&callback, &flow.state, timeout).await?;

    let scheme = if flow.domain.starts_with("127.0.0.1:") || flow.domain.starts_with("localhost:") {
        "http"
    } else {
        "https"
    };
    let token_url = format!("{scheme}://{}/oauth/token", flow.domain);
    let client = auth_http_client()?;
    let response = client
        .post(token_url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", flow.redirect_uri.as_str()),
            ("client_id", flow.client_id.as_str()),
            ("code_verifier", flow.code_verifier.as_str()),
        ])
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
            "openai token exchange failed with {status}: {body}"
        )));
    }

    let token: OpenAiTokenExchangeResponse =
        serde_json::from_str(&body).map_err(|err| AuthError::Parse(err.to_string()))?;
    let account_id = token
        .id_token
        .as_deref()
        .and_then(extract_openai_account_id_from_jwt)
        .or_else(|| extract_openai_account_id_from_jwt(&token.access_token));

    Ok(DeviceCodeFlowCredential {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_in_secs: token.expires_in,
        account_id,
    })
}

async fn wait_for_oauth_callback(
    callback: &CallbackRoute,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, AuthError> {
    let listener = TcpListener::bind(("127.0.0.1", callback.port))
        .await
        .map_err(|err| {
            AuthError::Network(format!(
                "failed to bind oauth callback port {}: {err}",
                callback.port
            ))
        })?;
    let deadline = tokio::time::Instant::now() + timeout.max(Duration::from_secs(1));

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(AuthError::OAuthTimeout);
        }
        let remaining = deadline - now;
        let accepted = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| AuthError::OAuthTimeout)?
            .map_err(|err| AuthError::Network(format!("oauth callback accept failed: {err}")))?;
        let (mut socket, _) = accepted;
        if let Some(code) =
            handle_oauth_callback_connection(&mut socket, expected_state, &callback.path).await?
        {
            return Ok(code);
        }
    }
}

async fn handle_oauth_callback_connection(
    socket: &mut tokio::net::TcpStream,
    expected_state: &str,
    expected_path: &str,
) -> Result<Option<String>, AuthError> {
    let mut buffer = [0_u8; 16 * 1024];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
        .await
        .map_err(|_| AuthError::Network("oauth callback read timed out".to_string()))?
        .map_err(|err| AuthError::Network(format!("oauth callback read failed: {err}")))?;
    if read == 0 {
        return Ok(None);
    }

    let request = String::from_utf8_lossy(&buffer[..read]);
    let first_line = request.lines().next().ok_or_else(|| {
        AuthError::Parse("oauth callback request was missing request line".to_string())
    })?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");

    if method != "GET" {
        write_callback_response(socket, "405 Method Not Allowed", "method not allowed").await?;
        return Ok(None);
    }

    let parsed_url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|err| AuthError::Parse(format!("oauth callback url parse failed: {err}")))?;
    if parsed_url.path() != expected_path {
        write_callback_response(socket, "404 Not Found", "not found").await?;
        return Ok(None);
    }

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    let mut error_description: Option<String> = None;
    for (key, value) in parsed_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(reason) = error {
        let detail = error_description.unwrap_or_else(|| "oauth authorization failed".to_string());
        write_callback_response(socket, "400 Bad Request", "authorization failed").await?;
        return Err(AuthError::OAuthFailed(format!("{reason}: {detail}")));
    }

    if state.as_deref() != Some(expected_state) {
        write_callback_response(socket, "400 Bad Request", "invalid oauth state").await?;
        return Err(AuthError::OAuthFailed(
            "oauth callback contained invalid state".to_string(),
        ));
    }

    let code = code.ok_or_else(|| {
        AuthError::Parse("oauth callback did not include authorization code".to_string())
    })?;
    write_callback_response(
        socket,
        "200 OK",
        "authorization complete; return to rustcode terminal",
    )
    .await?;
    Ok(Some(code))
}

async fn write_callback_response(
    socket: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> Result<(), AuthError> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| AuthError::Network(format!("oauth callback write failed: {err}")))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct CallbackRoute {
    port: u16,
    path: String,
}

fn callback_route_from_redirect_uri(redirect_uri: &str) -> Result<CallbackRoute, AuthError> {
    let parsed = reqwest::Url::parse(redirect_uri)
        .map_err(|err| AuthError::Validation(format!("invalid redirect uri: {err}")))?;
    let port = parsed.port_or_known_default().ok_or_else(|| {
        AuthError::Validation("redirect uri does not contain a callback port".to_string())
    })?;
    let path = if parsed.path().is_empty() {
        "/".to_string()
    } else {
        parsed.path().to_string()
    };
    Ok(CallbackRoute { port, path })
}

fn extract_openai_account_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload.as_bytes()).ok()?;
    let claims = serde_json::from_slice::<serde_json::Value>(&decoded).ok()?;

    claims
        .get("chatgpt_account_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|value| value.get("chatgpt_account_id"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            claims
                .get("organizations")
                .and_then(serde_json::Value::as_array)
                .and_then(|values| values.first())
                .and_then(|value| value.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn generate_oauth_state() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_verifier() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    (0..64)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn generate_pkce_code_challenge(verifier: &str) -> String {
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn deserialize_u64_string_or_number<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumberOrString {
        Number(u64),
        String(String),
    }

    let value = NumberOrString::deserialize(deserializer)?;
    match value {
        NumberOrString::Number(number) => Ok(number),
        NumberOrString::String(text) => {
            text.trim().parse::<u64>().map_err(serde::de::Error::custom)
        }
    }
}

pub fn normalize_domain(raw: &str) -> Result<String, AuthError> {
    if raw.trim().is_empty() {
        return Err(AuthError::Validation(
            "domain must not be empty".to_string(),
        ));
    }
    let trimmed = raw.trim();
    if trimmed.contains("://") {
        let parsed = reqwest::Url::parse(trimmed)
            .map_err(|err| AuthError::Validation(format!("invalid domain url: {err}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| AuthError::Validation("domain url is missing host".to_string()))?;
        return Ok(match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        });
    }

    let without_path = trimmed.split('/').next().unwrap_or_default();
    if without_path.is_empty() {
        return Err(AuthError::Validation(
            "domain must include host".to_string(),
        ));
    }
    Ok(without_path.to_string())
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
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;
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
    fn oauth_round_trip_and_remove() {
        let path = make_temp_file_path("auth-oauth-roundtrip");
        let store = AuthStore::with_path(path);

        store
            .set_oauth(
                "openai",
                "access-token",
                Some("refresh-token"),
                Some(1234),
                Some("account-1"),
            )
            .expect("set oauth should succeed");
        let value = store.get("openai").expect("get oauth should succeed");
        match value {
            Some(StoredCredential::OAuth {
                access_token,
                refresh_token,
                expires_at_unix,
                account_id,
                domain,
            }) => {
                assert_eq!(access_token, "access-token");
                assert_eq!(refresh_token.as_deref(), Some("refresh-token"));
                assert_eq!(expires_at_unix, Some(1234));
                assert_eq!(account_id.as_deref(), Some("account-1"));
                assert!(domain.is_none());
            }
            _ => panic!("expected oauth credential"),
        }
    }

    #[test]
    fn api_key_can_store_domain_metadata() {
        let path = make_temp_file_path("auth-api-key-domain");
        let store = AuthStore::with_path(path);

        store
            .set_api_key_with_domain(
                "github-copilot-enterprise",
                "enterprise-token",
                Some("github.example.com"),
            )
            .expect("set key should succeed");

        let value = store
            .get("github-copilot-enterprise")
            .expect("get should succeed");
        match value {
            Some(StoredCredential::ApiKey { key, domain }) => {
                assert_eq!(key, "enterprise-token");
                assert_eq!(domain.as_deref(), Some("github.example.com"));
            }
            _ => panic!("expected api key credential"),
        }
    }

    #[test]
    fn empty_auth_file_is_treated_as_default_store() {
        let path = make_temp_file_path("auth-empty-file");
        std::fs::write(&path, "").expect("must write empty file");
        let store = AuthStore::with_path(path);
        let providers = store.providers().expect("providers should load");
        assert!(providers.is_empty());
    }

    #[test]
    fn oauth_method_mapping_is_provider_specific() {
        assert_eq!(
            methods_for_provider("openai"),
            vec![
                AuthMethod::OAuthDeviceCode,
                AuthMethod::OAuthBrowser,
                AuthMethod::ApiKey,
            ]
        );
        assert_eq!(
            methods_for_provider("github-copilot"),
            vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
        );
        assert_eq!(
            methods_for_provider("gitlab"),
            vec![AuthMethod::OAuthBrowser, AuthMethod::ApiKey]
        );
        assert_eq!(methods_for_provider("openrouter"), vec![AuthMethod::ApiKey]);
    }

    #[test]
    fn oauth_hint_exists_for_supported_provider() {
        let hint = oauth_login_hint("gitlab").expect("hint must exist");
        assert!(hint.authorize_url.contains("gitlab"));
    }

    #[test]
    fn mcp_discovery_paths_include_canonical_and_path_scoped_candidates() {
        assert_eq!(
            mcp_discovery_paths("/"),
            vec!["/.well-known/oauth-authorization-server".to_string()]
        );
        assert_eq!(
            mcp_discovery_paths("/mcp"),
            vec![
                "/.well-known/oauth-authorization-server/mcp".to_string(),
                "/mcp/.well-known/oauth-authorization-server".to_string(),
                "/.well-known/oauth-authorization-server".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn discover_mcp_oauth_rejects_invalid_url() {
        let error = discover_mcp_oauth("not-a-url")
            .await
            .expect_err("invalid url should fail");
        assert!(error.to_string().contains("invalid MCP URL"));
    }

    #[test]
    fn mcp_browser_flow_requires_oauth_endpoints() {
        let discovery = McpOAuthDiscovery {
            supported: true,
            metadata_url: None,
            authorization_endpoint: None,
            token_endpoint: None,
        };
        let error = start_mcp_browser_oauth_flow(
            "github",
            "https://example.com/mcp",
            &discovery,
            "client-123",
            1458,
            &[],
        )
        .expect_err("missing endpoints should fail");
        assert!(error.to_string().contains("authorization_endpoint"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mcp_browser_flow_round_trip_with_mock_token_exchange() {
        let Some(callback_port) = allocate_port() else {
            eprintln!("skipping test: callback port bind is not permitted in this environment");
            return;
        };
        let Some(token_port) = allocate_port() else {
            eprintln!("skipping test: token port bind is not permitted in this environment");
            return;
        };
        let token_endpoint = format!("http://127.0.0.1:{token_port}/oauth/token");
        let authorize_endpoint = format!("http://127.0.0.1:{token_port}/oauth/authorize");
        let discovery = McpOAuthDiscovery {
            supported: true,
            metadata_url: Some(format!(
                "http://127.0.0.1:{token_port}/.well-known/oauth-authorization-server"
            )),
            authorization_endpoint: Some(authorize_endpoint),
            token_endpoint: Some(token_endpoint),
        };
        let capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let capture_clone = capture.clone();
        let Some(server) = spawn_mock_mcp_token_server(token_port, capture_clone) else {
            eprintln!("skipping test: token server bind is not permitted in this environment");
            return;
        };

        let flow = start_mcp_browser_oauth_flow(
            "github",
            "http://127.0.0.1:39443/mcp",
            &discovery,
            "client-123",
            callback_port,
            &["read".to_string(), "write".to_string()],
        )
        .expect("mcp browser flow should start");
        assert_eq!(flow.server_name, "github");
        assert_eq!(
            flow.redirect_uri,
            format!("http://127.0.0.1:{callback_port}/auth/callback")
        );
        let authorize_url =
            reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
        let state = authorize_url
            .query_pairs()
            .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
            .expect("state query parameter should exist");
        let scope = authorize_url
            .query_pairs()
            .find_map(|(key, value)| (key == "scope").then(|| value.into_owned()))
            .expect("scope query parameter should exist");
        assert_eq!(scope, "read write");

        let redirect_uri = flow.redirect_uri.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let callback_url = format!("{redirect_uri}?code=mcp-auth-code&state={state}");
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .no_proxy()
                .build()
                .expect("callback client should build");
            let _ = client.get(callback_url).send().await;
        });

        let credential =
            complete_mcp_browser_oauth_flow(&flow, Duration::from_secs(5), Some("shh"))
                .await
                .expect("flow should complete");
        assert_eq!(credential.access_token, "mcp-access-token");
        assert_eq!(
            credential.refresh_token.as_deref(),
            Some("mcp-refresh-token")
        );
        assert_eq!(credential.expires_in_secs, Some(3600));

        server.join().expect("mock mcp server should join");
        let body = capture.lock().expect("capture should lock").clone();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("client_id=client-123"));
        assert!(body.contains("code=mcp-auth-code"));
        assert!(body.contains("code_verifier="));
        assert!(body.contains("client_secret=shh"));
    }

    #[test]
    fn normalizes_domain_for_device_flow() {
        assert_eq!(
            normalize_domain("https://github.com/").expect("must normalize"),
            "github.com"
        );
        assert_eq!(
            normalize_domain("https://gitlab.example.com:8443/root/path").expect("must normalize"),
            "gitlab.example.com:8443"
        );
        assert_eq!(
            normalize_domain("company.ghe.com").expect("must normalize"),
            "company.ghe.com"
        );
    }

    #[test]
    fn parses_interval_from_string_or_number() {
        #[derive(Deserialize)]
        struct Holder {
            #[serde(deserialize_with = "deserialize_u64_string_or_number")]
            value: u64,
        }

        let parsed_number: Holder =
            serde_json::from_str(r#"{"value":5}"#).expect("number interval must parse");
        assert_eq!(parsed_number.value, 5);

        let parsed_string: Holder =
            serde_json::from_str(r#"{"value":"7"}"#).expect("string interval must parse");
        assert_eq!(parsed_string.value, 7);
    }

    #[tokio::test]
    async fn openai_browser_flow_builds_authorize_url() {
        let flow = start_browser_oauth_flow("openai", None, None, 1455)
            .await
            .expect("openai browser flow should start");
        assert_eq!(flow.provider, "openai");
        assert_eq!(flow.redirect_uri, "http://127.0.0.1:1455/auth/callback");
        assert!(flow
            .authorize_url
            .contains("https://auth.openai.com/oauth/authorize"));
        assert!(flow
            .authorize_url
            .contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(flow.authorize_url.contains("code_challenge_method=S256"));
    }

    #[tokio::test]
    async fn gitlab_browser_flow_uses_bundled_client_id_for_gitlab_com() {
        let flow = start_browser_oauth_flow("gitlab", None, None, 1456)
            .await
            .expect("gitlab.com browser flow should start");
        let authorize_url =
            reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
        let client_id = authorize_url
            .query_pairs()
            .find_map(|(key, value)| (key == "client_id").then(|| value.into_owned()))
            .expect("client_id query parameter should exist");
        assert_eq!(client_id, GITLAB_BUNDLED_CLIENT_ID);
    }

    #[tokio::test]
    async fn gitlab_browser_flow_requires_client_id_for_self_hosted() {
        let error = start_browser_oauth_flow("gitlab", Some("gitlab.example.com"), None, 1457)
            .await
            .expect_err("self-hosted gitlab flow should require client id");
        assert!(error.to_string().contains("GITLAB_OAUTH_CLIENT_ID"));
    }

    #[test]
    fn extracts_openai_account_id_from_jwt_claims() {
        let payload = serde_json::json!({
            "chatgpt_account_id": "acct_123"
        });
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        let token = format!("header.{encoded}.signature");
        assert_eq!(
            extract_openai_account_id_from_jwt(&token).as_deref(),
            Some("acct_123")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gitlab_browser_flow_round_trip_with_mock_token_exchange() {
        let Some(callback_port) = allocate_port() else {
            eprintln!("skipping test: callback port bind is not permitted in this environment");
            return;
        };
        let Some(token_port) = allocate_port() else {
            eprintln!("skipping test: token port bind is not permitted in this environment");
            return;
        };
        let token_domain = format!("127.0.0.1:{token_port}");
        let capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let capture_clone = capture.clone();
        let Some(server) = spawn_mock_gitlab_token_server(token_port, capture_clone) else {
            eprintln!("skipping test: token server bind is not permitted in this environment");
            return;
        };

        let flow = start_browser_oauth_flow(
            "gitlab",
            Some(&token_domain),
            Some("client-123"),
            callback_port,
        )
        .await
        .expect("flow should start");
        let authorize_url =
            reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
        let state = authorize_url
            .query_pairs()
            .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
            .expect("state query parameter should exist");
        assert!(flow.authorize_url.contains("code_challenge="));

        let redirect_uri = flow.redirect_uri.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let callback_url = format!("{redirect_uri}?code=auth-code-1&state={state}");
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .no_proxy()
                .build()
                .expect("callback client should build");
            let _ = client.get(callback_url).send().await;
        });

        let credential = complete_browser_oauth_flow(&flow, Duration::from_secs(5), None)
            .await
            .expect("flow should complete");
        assert_eq!(credential.access_token, "gitlab-access-token");
        assert_eq!(
            credential.refresh_token.as_deref(),
            Some("gitlab-refresh-token")
        );
        assert_eq!(credential.expires_in_secs, Some(1800));

        server.join().expect("mock gitlab server should join");
        let body = capture.lock().expect("capture should lock").clone();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("client_id=client-123"));
        assert!(body.contains("code=auth-code-1"));
        assert!(body.contains("code_verifier="));
    }

    fn make_temp_file_path(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("monotonic time")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("rustcode-auth-{name}-{pid}-{now}.json"))
    }

    fn allocate_port() -> Option<u16> {
        let listener = match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
            Err(err) => panic!("must bind random port: {err}"),
        };
        let port = listener.local_addr().expect("must read local addr").port();
        drop(listener);
        Some(port)
    }

    fn spawn_mock_gitlab_token_server(
        port: u16,
        body_capture: std::sync::Arc<std::sync::Mutex<String>>,
    ) -> Option<std::thread::JoinHandle<()>> {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
            Err(err) => panic!("must bind token port: {err}"),
        };
        Some(thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("must configure listener nonblocking");
            // If the token exchange never happens (e.g., upstream request fails), we must not hang
            // the entire test suite waiting on accept/join.
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut accepted: Option<TcpStream> = None;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        accepted = Some(stream);
                        break;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => panic!("must accept token request: {err}"),
                }
            }
            let Some(mut stream) = accepted else {
                // Let the assertions in the test fail on empty capture instead of hanging forever.
                return;
            };
            let request = read_http_request(&mut stream);
            let body = request
                .split("\r\n\r\n")
                .nth(1)
                .unwrap_or_default()
                .to_string();
            *body_capture
                .lock()
                .expect("body capture must lock for write") = body;
            let response_body = r#"{"access_token":"gitlab-access-token","refresh_token":"gitlab-refresh-token","expires_in":1800}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("must write token response");
        }))
    }

    fn spawn_mock_mcp_token_server(
        port: u16,
        body_capture: std::sync::Arc<std::sync::Mutex<String>>,
    ) -> Option<std::thread::JoinHandle<()>> {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
            Err(err) => panic!("must bind token port: {err}"),
        };
        Some(thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("must configure listener nonblocking");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut accepted: Option<TcpStream> = None;
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        accepted = Some(stream);
                        break;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(err) => panic!("must accept token request: {err}"),
                }
            }
            let Some(mut stream) = accepted else {
                return;
            };
            let request = read_http_request(&mut stream);
            let body = request
                .split("\r\n\r\n")
                .nth(1)
                .unwrap_or_default()
                .to_string();
            *body_capture
                .lock()
                .expect("body capture must lock for write") = body;
            let response_body = r#"{"access_token":"mcp-access-token","refresh_token":"mcp-refresh-token","expires_in":3600}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .expect("must write token response");
        }))
    }

    fn read_http_request(stream: &mut TcpStream) -> String {
        let mut buffer = [0_u8; 16 * 1024];
        let mut request = Vec::new();
        let mut content_length = 0_usize;
        let mut header_end_index: Option<usize> = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if std::time::Instant::now() > deadline {
                break;
            }
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    request.extend_from_slice(&buffer[..read]);
                    if header_end_index.is_none() {
                        if let Some(index) =
                            request.windows(4).position(|chunk| chunk == b"\r\n\r\n")
                        {
                            let end = index + 4;
                            header_end_index = Some(end);
                            let headers = String::from_utf8_lossy(&request[..end]);
                            for line in headers.lines() {
                                let lower = line.to_ascii_lowercase();
                                if let Some(value) = lower.strip_prefix("content-length:") {
                                    content_length = value.trim().parse::<usize>().unwrap_or(0);
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(end) = header_end_index {
                        if request.len() >= end + content_length {
                            break;
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("must read request: {err}"),
            }
        }
        String::from_utf8(request).expect("request must be utf8")
    }
}
