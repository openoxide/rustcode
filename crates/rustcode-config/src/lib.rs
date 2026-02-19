use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rustcode_core::config::ResolvedConfig;
use rustcode_core::error::ConfigError;
use rustcode_core::permissions::PermissionRule;
use serde::Deserialize;

mod mcp_edit;
mod merge;
mod paths;
mod validation;

pub use mcp_edit::{edit_mcp_server, remove_mcp_server, ConfigEditScope};
use paths::{
    parse_boolish_env, resolve_global_config_path, resolve_project_config_path,
    resolve_user_config_path,
};

#[derive(Debug, Clone)]
pub struct ConfigSources {
    pub cwd: PathBuf,
    pub profile_override: Option<String>,
    pub model_override: Option<String>,
    pub allow_network_override: Option<bool>,
    pub llm_provider_override: Option<String>,
    pub llm_base_url_override: Option<String>,
    pub llm_api_key_env_override: Option<String>,
    pub global_config_path: Option<PathBuf>,
    pub user_config_path: Option<PathBuf>,
    pub project_config_path: Option<PathBuf>,
    pub trust_project: bool,
    pub read_process_env: bool,
}

impl ConfigSources {
    #[must_use]
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            profile_override: None,
            model_override: None,
            allow_network_override: None,
            llm_provider_override: None,
            llm_base_url_override: None,
            llm_api_key_env_override: None,
            global_config_path: None,
            user_config_path: None,
            project_config_path: None,
            trust_project: false,
            read_process_env: true,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileConfig {
    pub profile: Option<String>,
    pub model: Option<String>,
    pub llm: Option<LlmConfig>,
    pub enabled_providers: Option<Vec<String>>,
    pub disabled_providers: Option<Vec<String>>,
    pub mcp: Option<McpConfig>,
    pub allow_network: Option<bool>,
    pub plugins: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub policy: Option<PolicyConfig>,
    pub permissions: Option<Vec<PermissionRule>>,
    pub trust: Option<TrustConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LlmConfig {
    pub provider: Option<String>,
    #[serde(alias = "baseURL")]
    pub base_url: Option<String>,
    #[serde(alias = "apiKeyEnv")]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct McpConfig {
    pub servers: Option<BTreeMap<String, McpServerFileConfig>>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct McpServerFileConfig {
    pub url: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub oauth: Option<McpOAuthFileConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum McpOAuthFileConfig {
    Enabled(bool),
    Settings(McpOAuthFileSettings),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct McpOAuthFileSettings {
    pub enabled: Option<bool>,
    pub client_id: Option<String>,
    pub client_secret_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct TrustConfig {
    pub projects: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PolicyConfig {
    pub backend_selection: Option<BackendSelectionPolicyConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct BackendSelectionPolicyConfig {
    #[serde(alias = "providerAgnosticWeight")]
    pub provider_agnostic_weight: Option<i32>,
    #[serde(alias = "automationSkillsWeight")]
    pub automation_skills_weight: Option<i32>,
    #[serde(alias = "openSourceWeight")]
    pub open_source_weight: Option<i32>,
    #[serde(alias = "lspSupportWeight")]
    pub lsp_support_weight: Option<i32>,
    #[serde(alias = "privacyWeight")]
    pub privacy_weight: Option<i32>,
    #[serde(alias = "subscriptionPenalty")]
    pub subscription_penalty: Option<i32>,
}

pub struct ConfigLoader;

impl ConfigLoader {
    /// Load and validate the effective configuration based on the provided sources.
    ///
    /// # Errors
    /// Returns `ConfigError` when configuration layers cannot be read/parsed/validated, or when
    /// a project config exists but is not trusted.
    pub fn load(sources: &ConfigSources) -> Result<ResolvedConfig, ConfigError> {
        let mut cfg = ResolvedConfig {
            workspace_root: sources.cwd.clone(),
            ..ResolvedConfig::default()
        };

        let global_config_path = resolve_global_config_path(sources);
        let user_config_path = resolve_user_config_path(sources);
        let project_config_path = resolve_project_config_path(sources);

        let global_cfg = read_file_config(&global_config_path)?;
        if let Some(file_cfg) = global_cfg.as_ref() {
            merge::apply_file(&mut cfg, file_cfg);
        }

        let user_cfg = read_file_config(&user_config_path)?;
        if let Some(file_cfg) = user_cfg.as_ref() {
            merge::apply_file(&mut cfg, file_cfg);
        }

        let trusted_projects =
            trusted_project_set(&[global_cfg.as_ref(), user_cfg.as_ref()], &sources.cwd);

        apply_project_config(&mut cfg, sources, &project_config_path, &trusted_projects)?;
        apply_env_overrides(&mut cfg, sources);
        apply_cli_overrides(&mut cfg, sources);

        validation::validate(&cfg)?;
        Ok(cfg)
    }
}

fn apply_project_config(
    cfg: &mut ResolvedConfig,
    sources: &ConfigSources,
    project_config_path: &Path,
    trusted_projects: &BTreeSet<PathBuf>,
) -> Result<(), ConfigError> {
    let project_cfg = read_file_config(project_config_path)?;
    if let Some(file_cfg) = project_cfg.as_ref() {
        let env_trust = sources.read_process_env && env_truthy("RUSTCODE_TRUST_PROJECT");
        let project_explicitly_trusted = sources.trust_project || env_trust;
        let project_in_trust_list = trusted_projects.contains(&normalize_path(&sources.cwd));
        let project_trusted = project_explicitly_trusted || project_in_trust_list;

        if !project_trusted {
            return Err(ConfigError::Trust(format!(
                "project config {} exists but {} is not trusted; pass --trust-project-config, set RUSTCODE_TRUST_PROJECT=1, or add the path to [trust].projects",
                project_config_path.display(),
                sources.cwd.display()
            )));
        }

        cfg.project_config_path = Some(project_config_path.to_path_buf());
        cfg.project_config_trusted = true;
        merge::apply_file(cfg, file_cfg);
    }
    Ok(())
}

fn apply_env_overrides(cfg: &mut ResolvedConfig, sources: &ConfigSources) {
    if !sources.read_process_env {
        return;
    }
    if let Ok(profile) = std::env::var("RUSTCODE_PROFILE") {
        cfg.profile = profile;
    }
    if let Ok(model) = std::env::var("RUSTCODE_MODEL") {
        cfg.model = model;
    }
    if let Ok(allow_network) = std::env::var("RUSTCODE_ALLOW_NETWORK") {
        if let Ok(val) = parse_boolish_env("RUSTCODE_ALLOW_NETWORK", &allow_network) {
            cfg.allow_network = val;
        }
    }
    if let Ok(llm_provider) = std::env::var("RUSTCODE_LLM_PROVIDER") {
        cfg.llm_provider = llm_provider;
    }
    if let Ok(llm_base_url) = std::env::var("RUSTCODE_LLM_BASE_URL") {
        cfg.llm_base_url = Some(llm_base_url);
    }
    if let Ok(llm_api_key_env) = std::env::var("RUSTCODE_LLM_API_KEY_ENV") {
        cfg.llm_api_key_env = Some(llm_api_key_env);
    }
}

fn apply_cli_overrides(cfg: &mut ResolvedConfig, sources: &ConfigSources) {
    if let Some(profile) = &sources.profile_override {
        cfg.profile.clone_from(profile);
    }
    if let Some(model) = &sources.model_override {
        cfg.model.clone_from(model);
    }
    if let Some(allow_network) = sources.allow_network_override {
        cfg.allow_network = allow_network;
    }
    if let Some(provider) = &sources.llm_provider_override {
        cfg.llm_provider.clone_from(provider);
    }
    if let Some(base_url) = &sources.llm_base_url_override {
        cfg.llm_base_url = Some(base_url.clone());
    }
    if let Some(api_key_env) = &sources.llm_api_key_env_override {
        cfg.llm_api_key_env = Some(api_key_env.clone());
    }
}

fn read_file_config(path: &Path) -> Result<Option<FileConfig>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .map_err(|err| ConfigError::Read(format!("{}: {err}", path.display())))?;
    let parsed = toml::from_str::<FileConfig>(&raw)
        .map_err(|err| ConfigError::Parse(format!("{}: {err}", path.display())))?;
    Ok(Some(parsed))
}

fn trusted_project_set(configs: &[Option<&FileConfig>], cwd: &Path) -> BTreeSet<PathBuf> {
    let mut trusted = BTreeSet::new();

    for cfg in configs.iter().flatten().copied() {
        if let Some(trust) = &cfg.trust {
            if let Some(projects) = &trust.projects {
                for path in projects {
                    let raw = PathBuf::from(path);
                    let full = if raw.is_absolute() {
                        raw
                    } else {
                        cwd.join(raw)
                    };
                    trusted.insert(normalize_path(&full));
                }
            }
        }
    }

    trusted
}

fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn env_truthy(var_name: &str) -> bool {
    let Ok(raw) = std::env::var(var_name) else {
        return false;
    };

    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests;
