use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub profile: String,
    pub workspace_root: PathBuf,
    pub allow_network: bool,
    pub model: String,
    pub llm_provider: String,
    pub llm_base_url: Option<String>,
    pub llm_api_key_env: Option<String>,
    pub plugins: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub backend_selection: BackendSelectionPolicy,
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
            plugins: Vec::new(),
            env: BTreeMap::new(),
            backend_selection: BackendSelectionPolicy::default(),
            project_config_path: None,
            project_config_trusted: false,
        }
    }
}
