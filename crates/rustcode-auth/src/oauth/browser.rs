use std::collections::BTreeMap;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde::Deserialize;
use sha2::Digest;

use crate::{auth_http_client, AuthError, McpOAuthDiscovery};

use super::callback::{callback_route_from_redirect_uri, wait_for_oauth_callback};
use super::device::DeviceCodeFlowCredential;

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const GITLAB_BUNDLED_CLIENT_ID: &str =
    "1d89f9fdb23ee96d4e603201f6861dab6e143c5c3c00469a018a2d94bdc03d4e";
const GITLAB_DEFAULT_SCOPES: &str = "api read_user read_repository";
const OPENAI_BROWSER_SCOPES: &str = "openid profile email offline_access";
const GITLAB_OAUTH_CALLBACK_PATH: &str = "/callback";
const MCP_OAUTH_CALLBACK_PATH: &str = "/auth/callback";
const OPENAI_OAUTH_CALLBACK_PATH: &str = "/auth/callback";

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

/// Build a browser OAuth flow challenge (authorize URL + local callback redirect).
///
/// This does not perform any network requests; it only constructs the authorization URL and
/// PKCE parameters for the given provider.
///
/// # Errors
/// Returns `AuthError` if input validation fails or the provider is unsupported.
pub fn start_browser_oauth_flow(
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

/// Complete a browser OAuth flow by waiting for the local callback and exchanging the code.
///
/// # Errors
/// Returns `AuthError` if the callback is not received in time or token exchange fails.
pub async fn complete_browser_oauth_flow(
    flow: &BrowserOAuthFlowStart,
    timeout: Duration,
    client_secret: Option<&str>,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    match flow.provider.as_str() {
        "openai" => Box::pin(complete_openai_browser_oauth_flow(flow, timeout)).await,
        "gitlab" => {
            Box::pin(complete_gitlab_browser_oauth_flow(
                flow,
                timeout,
                client_secret,
            ))
            .await
        }
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support browser oauth completion"
        ))),
    }
}

/// Build an MCP browser OAuth flow challenge (authorize URL + local callback redirect).
///
/// # Errors
/// Returns `AuthError` if discovery metadata is missing required endpoints or input validation fails.
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

/// Complete an MCP browser OAuth flow by waiting for the local callback and exchanging the code.
///
/// # Errors
/// Returns `AuthError` if the callback is not received in time or token exchange fails.
pub async fn complete_mcp_browser_oauth_flow(
    flow: &McpBrowserOAuthFlowStart,
    timeout: Duration,
    client_secret: Option<&str>,
) -> Result<DeviceCodeFlowCredential, AuthError> {
    let callback = callback_route_from_redirect_uri(&flow.redirect_uri)?;
    let code = Box::pin(wait_for_oauth_callback(&callback, &flow.state, timeout)).await?;

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

#[derive(Debug, Deserialize)]
struct OpenAiTokenExchangeResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
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
    let scheme = oauth_scheme_for_domain(&normalized_domain);
    let issuer = format!("{scheme}://{normalized_domain}");
    let redirect_uri = format!("http://localhost:{callback_port}{OPENAI_OAUTH_CALLBACK_PATH}");
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

    let scheme = oauth_scheme_for_domain(&normalized_domain);
    let mut authorize_url = reqwest::Url::parse(&format!(
        "{scheme}://{normalized_domain}/oauth/authorize"
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
    let code = Box::pin(wait_for_oauth_callback(&callback, &flow.state, timeout)).await?;

    let scheme = oauth_scheme_for_domain(&flow.domain);
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
    let code = Box::pin(wait_for_oauth_callback(&callback, &flow.state, timeout)).await?;

    let scheme = oauth_scheme_for_domain(&flow.domain);
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

pub(crate) fn extract_openai_account_id_from_jwt(token: &str) -> Option<String> {
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

fn oauth_scheme_for_domain(domain: &str) -> &'static str {
    if is_loopback_domain(domain) {
        "http"
    } else {
        "https"
    }
}

fn is_loopback_domain(domain: &str) -> bool {
    let candidate = if domain.contains("://") {
        domain.to_string()
    } else {
        format!("http://{domain}")
    };
    let Ok(parsed) = reqwest::Url::parse(&candidate) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Normalize a user-provided domain or URL into a host[:port] form.
///
/// # Errors
/// Returns `AuthError` if the input cannot be parsed into a valid domain.
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
