// ── GitHub Copilot authentication plugin ──────────────────────────────
//
// Implements the device code OAuth flow for GitHub Copilot.
// After authorization, injects the appropriate bearer token and
// Copilot-specific headers into LLM requests.

use async_trait::async_trait;
use rustcode_core::context::CommandContext;
use rustcode_core::event::Event;

use crate::{Plugin, PluginError};

/// GitHub OAuth App client ID registered for Copilot CLI tools.
const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

/// Default GitHub domain for public accounts.
const DEFAULT_DOMAIN: &str = "github.com";

/// Copilot-specific request headers.
const COPILOT_INTENT_HEADER: &str = "Openai-Intent";
const COPILOT_INTENT_VALUE: &str = "conversation-edits";

/// GitHub Copilot plugin — handles device code OAuth and request signing.
pub struct CopilotPlugin {
    /// GitHub domain (e.g. "github.com" or "company.ghe.com" for Enterprise).
    domain: String,
    /// Provider ID — "github-copilot" or "github-copilot-enterprise".
    provider_id: String,
}

impl CopilotPlugin {
    /// Create a plugin instance for public GitHub Copilot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            domain: DEFAULT_DOMAIN.to_string(),
            provider_id: "github-copilot".to_string(),
        }
    }

    /// Create a plugin instance for GitHub Enterprise Copilot.
    #[must_use]
    pub fn enterprise(domain: &str) -> Self {
        let normalized = domain
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        Self {
            domain: normalized.to_string(),
            provider_id: "github-copilot-enterprise".to_string(),
        }
    }

    /// URL for initiating the device code flow.
    #[must_use]
    pub fn device_code_url(&self) -> String {
        format!("https://{}/login/device/code", self.domain)
    }

    /// URL for exchanging device code for access token.
    #[must_use]
    pub fn access_token_url(&self) -> String {
        format!("https://{}/login/oauth/access_token", self.domain)
    }

    /// Copilot API base URL (differs for Enterprise).
    #[must_use]
    pub fn api_base_url(&self) -> Option<String> {
        if self.domain != DEFAULT_DOMAIN {
            Some(format!("https://copilot-api.{}", self.domain))
        } else {
            None
        }
    }

    /// Returns the provider ID for auth store lookups.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Returns the GitHub OAuth client ID.
    #[must_use]
    pub fn client_id(&self) -> &str {
        GITHUB_CLIENT_ID
    }
}

impl Default for CopilotPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// Device code flow initiation response from GitHub.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceCodeResponse {
    /// URI to show the user for verification.
    pub verification_uri: String,
    /// Code the user must enter at the verification URI.
    pub user_code: String,
    /// Device code used for polling.
    pub device_code: String,
    /// Polling interval in seconds.
    pub interval: u64,
}

/// Polling response from the access token endpoint.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenPollResponse {
    /// Access token (present on success).
    pub access_token: Option<String>,
    /// Error code (e.g. "authorization_pending", "slow_down").
    pub error: Option<String>,
    /// Server-suggested polling interval (for slow_down).
    pub interval: Option<u64>,
}

/// Build the Copilot-specific HTTP headers for LLM requests.
#[must_use]
pub fn copilot_request_headers(token: &str, is_agent: bool) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Authorization".to_string(), format!("Bearer {token}")),
        (COPILOT_INTENT_HEADER.to_string(), COPILOT_INTENT_VALUE.to_string()),
    ];

    let initiator = if is_agent { "agent" } else { "user" };
    headers.push(("x-initiator".to_string(), initiator.to_string()));

    headers
}

#[async_trait]
impl Plugin for CopilotPlugin {
    fn name(&self) -> &'static str {
        "copilot"
    }

    async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
        // Auth header injection is handled at the provider level via
        // `copilot_request_headers()`, not through event hooks.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_urls() {
        let plugin = CopilotPlugin::new();
        assert_eq!(plugin.device_code_url(), "https://github.com/login/device/code");
        assert_eq!(plugin.access_token_url(), "https://github.com/login/oauth/access_token");
        assert!(plugin.api_base_url().is_none());
        assert_eq!(plugin.provider_id(), "github-copilot");
    }

    #[test]
    fn enterprise_urls() {
        let plugin = CopilotPlugin::enterprise("https://company.ghe.com/");
        assert_eq!(plugin.device_code_url(), "https://company.ghe.com/login/device/code");
        assert_eq!(
            plugin.api_base_url(),
            Some("https://copilot-api.company.ghe.com".to_string())
        );
        assert_eq!(plugin.provider_id(), "github-copilot-enterprise");
    }

    #[test]
    fn request_headers_user() {
        let headers = copilot_request_headers("test-token", false);
        assert!(headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer test-token"));
        assert!(headers.iter().any(|(k, v)| k == "x-initiator" && v == "user"));
    }

    #[test]
    fn request_headers_agent() {
        let headers = copilot_request_headers("test-token", true);
        assert!(headers.iter().any(|(k, v)| k == "x-initiator" && v == "agent"));
    }
}
