// ── OpenAI Codex authentication plugin ────────────────────────────────
//
// Implements the PKCE browser OAuth flow for OpenAI Codex.
// After authorization, injects bearer tokens and handles token refresh.

use async_trait::async_trait;
use rustcode_core::context::CommandContext;
use rustcode_core::event::Event;

use crate::{Plugin, PluginError};

/// `OpenAI` OAuth client ID for Codex CLI.
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// `OpenAI` auth issuer.
const ISSUER: &str = "https://auth.openai.com";

/// Codex API endpoint.
const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Local callback port for PKCE OAuth.
const OAUTH_CALLBACK_PORT: u16 = 1455;

/// `OpenAI` Codex plugin — handles PKCE OAuth and request signing.
pub struct CodexPlugin;

impl CodexPlugin {
    /// Create a new Codex plugin instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Returns the OAuth client ID.
    #[must_use]
    pub fn client_id(&self) -> &str {
        CODEX_CLIENT_ID
    }

    /// Returns the OAuth issuer URL.
    #[must_use]
    pub fn issuer(&self) -> &str {
        ISSUER
    }

    /// Returns the Codex API endpoint.
    #[must_use]
    pub fn api_endpoint(&self) -> &str {
        CODEX_API_ENDPOINT
    }

    /// Returns the local OAuth callback port.
    #[must_use]
    pub fn callback_port(&self) -> u16 {
        OAUTH_CALLBACK_PORT
    }

    /// Build the authorization URL for PKCE flow.
    #[must_use]
    pub fn authorize_url(&self, redirect_uri: &str, code_challenge: &str, state: &str) -> String {
        format!(
            "{issuer}/authorize?\
            client_id={client_id}\
            &redirect_uri={redirect_uri}\
            &response_type=code\
            &code_challenge={code_challenge}\
            &code_challenge_method=S256\
            &state={state}\
            &scope=openid+profile+email+offline_access",
            issuer = ISSUER,
            client_id = CODEX_CLIENT_ID,
            redirect_uri = urlencoding::encode(redirect_uri),
            code_challenge = code_challenge,
            state = state,
        )
    }

    /// Returns the token exchange endpoint URL.
    #[must_use]
    pub fn token_endpoint(&self) -> String {
        format!("{ISSUER}/oauth/token")
    }

    /// Returns the redirect URI for the local callback server.
    #[must_use]
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{OAUTH_CALLBACK_PORT}/callback")
    }
}

impl Default for CodexPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// PKCE code pair for OAuth flow.
#[derive(Debug, Clone)]
pub struct PkceCodes {
    /// Random verifier string.
    pub verifier: String,
    /// SHA-256 hash of verifier, base64url-encoded.
    pub challenge: String,
}

/// Generate a PKCE code pair for the OAuth flow.
///
/// Uses the system random source for the verifier and computes the
/// SHA-256 challenge.
#[must_use]
pub fn generate_pkce() -> PkceCodes {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand::Rng;
    use sha2::{Digest, Sha256};

    let mut rng = rand::rng();
    let verifier: String = (0..64)
        .map(|_| {
            let idx = rng.random_range(0..62);

            if idx < 26 {
                (b'a' + idx) as char
            } else if idx < 52 {
                (b'A' + idx - 26) as char
            } else {
                (b'0' + idx - 52) as char
            }
        })
        .collect();

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    PkceCodes {
        verifier,
        challenge,
    }
}

/// Token response from the `OpenAI` token endpoint.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenResponse {
    /// JWT ID token.
    pub id_token: Option<String>,
    /// Bearer access token.
    pub access_token: String,
    /// Refresh token for getting new access tokens.
    pub refresh_token: Option<String>,
    /// Token lifetime in seconds.
    pub expires_in: Option<u64>,
}

/// Build Codex-specific HTTP headers for LLM requests.
#[must_use]
pub fn codex_request_headers(access_token: &str) -> Vec<(String, String)> {
    vec![(
        "Authorization".to_string(),
        format!("Bearer {access_token}"),
    )]
}

#[async_trait]
impl Plugin for CodexPlugin {
    fn name(&self) -> &'static str {
        "codex"
    }

    async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
        // Auth is handled at the provider level via `codex_request_headers()`.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_constants() {
        let plugin = CodexPlugin::new();
        assert_eq!(plugin.client_id(), "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(plugin.issuer(), "https://auth.openai.com");
        assert_eq!(plugin.callback_port(), 1455);
    }

    #[test]
    fn authorize_url_format() {
        let plugin = CodexPlugin::new();
        let url = plugin.authorize_url(
            "http://localhost:1455/callback",
            "test-challenge",
            "test-state",
        );
        assert!(url.starts_with("https://auth.openai.com/authorize?"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("code_challenge=test-challenge"));
        assert!(url.contains("state=test-state"));
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn token_endpoint() {
        let plugin = CodexPlugin::new();
        assert_eq!(
            plugin.token_endpoint(),
            "https://auth.openai.com/oauth/token"
        );
    }

    #[test]
    fn pkce_generation() {
        let codes = generate_pkce();
        assert_eq!(codes.verifier.len(), 64);
        assert!(!codes.challenge.is_empty());
        // Challenge should be different from verifier (it's a hash)
        assert_ne!(codes.verifier, codes.challenge);
    }

    #[test]
    fn request_headers() {
        let headers = codex_request_headers("test-access-token");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Authorization");
        assert_eq!(headers[0].1, "Bearer test-access-token");
    }
}
