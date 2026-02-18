use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer};
use tokio::time::sleep;

use crate::{auth_http_client, AuthError};

const OPENAI_ISSUER: &str = "https://auth.openai.com";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeFlowCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<u64>,
    pub account_id: Option<String>,
}

/// Start an OAuth device-code flow for the given provider.
///
/// # Errors
/// Returns `AuthError` if the provider does not support this flow, input is invalid, or network requests fail.
pub async fn start_device_code_flow(
    provider: &str,
    domain: Option<&str>,
) -> Result<DeviceCodeFlowStart, AuthError> {
    match provider {
        "openai" => start_openai_device_code(provider).await,
        "github-copilot" | "github-copilot-enterprise" => {
            let normalized_domain = super::normalize_domain(domain.unwrap_or("github.com"))?;
            start_github_device_code(provider, &normalized_domain).await
        }
        other => Err(AuthError::Validation(format!(
            "provider {other} does not support device code flow"
        ))),
    }
}

/// Poll a previously-started device-code flow until an access token is issued.
///
/// # Errors
/// Returns `AuthError` if the flow expires, times out, or the provider returns an error.
pub async fn poll_device_code_flow_for_api_key(
    flow: &DeviceCodeFlowStart,
    timeout: Duration,
) -> Result<String, AuthError> {
    let credential = poll_device_code_flow_for_credential(flow, timeout).await?;
    Ok(credential.access_token)
}

/// Poll a previously-started device-code flow until a credential is issued.
///
/// # Errors
/// Returns `AuthError` if the flow expires, times out, or the provider returns an error.
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
}

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

pub(crate) fn deserialize_u64_string_or_number<'de, D>(deserializer: D) -> Result<u64, D::Error>
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
