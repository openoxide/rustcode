use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::permissions::PermissionRule;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub profile: String,
    pub workspace_root: PathBuf,
    pub allow_network: bool,
    pub model: String,
    pub llm_provider: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key_env: Option<String>,
    // Provider filtering aligns with OpenCode semantics:
    // - if `enabled_providers` is set, only those providers are considered available
    // - `disabled_providers` always excludes providers
    pub enabled_providers: Option<BTreeSet<String>>,
    pub disabled_providers: BTreeSet<String>,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    pub plugins: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub backend_selection: BackendSelectionPolicy,
    pub permission_rules: Vec<PermissionRule>,
    pub project_config_path: Option<PathBuf>,
    pub project_config_trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSelectionPolicy {
    pub provider_agnostic_weight: i32,
    pub automation_skills_weight: i32,
    pub open_source_weight: i32,
    pub lsp_support_weight: i32,
    pub privacy_weight: i32,
    pub subscription_penalty: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub oauth: McpOAuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthConfig {
    pub enabled: bool,
    pub client_id: Option<String>,
    pub client_secret_env: Option<String>,
}

impl Default for McpOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: None,
            client_secret_env: None,
        }
    }
}

impl Default for BackendSelectionPolicy {
    fn default() -> Self {
        Self {
            provider_agnostic_weight: 2,
            automation_skills_weight: 3,
            open_source_weight: 1,
            lsp_support_weight: 1,
            privacy_weight: 2,
            subscription_penalty: 1,
        }
    }
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            workspace_root: PathBuf::from("."),
            allow_network: false,
            model: "default".to_string(),
            llm_provider: "null".to_string(),
            llm_base_url: None,
            llm_api_key_env: None,
            enabled_providers: None,
            disabled_providers: BTreeSet::new(),
            mcp_servers: BTreeMap::new(),
            plugins: Vec::new(),
            env: BTreeMap::new(),
            backend_selection: BackendSelectionPolicy::default(),
            permission_rules: Vec::new(),
            project_config_path: None,
            project_config_trusted: false,
        }
    }
}

impl ResolvedConfig {
    #[must_use]
    pub fn provider_allowed(&self, provider_id: &str) -> bool {
        if provider_id.eq_ignore_ascii_case("null") {
            return true;
        }
        if self
            .disabled_providers
            .iter()
            .any(|value| value.eq_ignore_ascii_case(provider_id))
        {
            return false;
        }
        match self.enabled_providers.as_ref() {
            None => true,
            Some(enabled) => enabled
                .iter()
                .any(|value| value.eq_ignore_ascii_case(provider_id)),
        }
    }
}
