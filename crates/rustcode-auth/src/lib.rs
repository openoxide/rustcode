use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
            "Complete ChatGPT authorization flow, then store the resulting token with `rustcode auth set-key openai --from-env <ENV_VAR>`.",
        ),
        "github-copilot" | "github-copilot-enterprise" => (
            "https://github.com/login/device",
            "Complete GitHub device login, then store the resulting token with `rustcode auth set-key <provider> --from-env <ENV_VAR>`.",
        ),
        "gitlab" => (
            "https://gitlab.com/oauth/authorize",
            "Complete GitLab OAuth login in browser, then store the resulting token with `rustcode auth set-key <provider> --from-env <ENV_VAR>`.",
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

    fn make_temp_file_path(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("monotonic time")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("rustcode-auth-{name}-{pid}-{now}.json"))
    }
}
