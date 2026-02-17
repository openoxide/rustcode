use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rustcode_core::config::{
    BackendSelectionPolicy, McpOAuthConfig, McpServerConfig, ResolvedConfig,
};
use rustcode_core::error::ConfigError;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct ConfigSources {
    pub cwd: PathBuf,
    pub profile_override: Option<String>,
    pub model_override: Option<String>,
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
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            profile_override: None,
            model_override: None,
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
struct FileConfig {
    profile: Option<String>,
    model: Option<String>,
    llm: Option<LlmConfig>,
    mcp: Option<McpConfig>,
    allow_network: Option<bool>,
    plugins: Option<Vec<String>>,
    env: Option<BTreeMap<String, String>>,
    policy: Option<PolicyConfig>,
    trust: Option<TrustConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct LlmConfig {
    provider: Option<String>,
    #[serde(alias = "baseURL")]
    base_url: Option<String>,
    #[serde(alias = "apiKeyEnv")]
    api_key_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct McpConfig {
    servers: Option<BTreeMap<String, McpServerFileConfig>>,
}

#[derive(Debug, Default, Deserialize)]
struct McpServerFileConfig {
    url: Option<String>,
    oauth: Option<McpOAuthFileConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum McpOAuthFileConfig {
    Enabled(bool),
    Settings(McpOAuthFileSettings),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct McpOAuthFileSettings {
    enabled: Option<bool>,
    client_id: Option<String>,
    client_secret_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TrustConfig {
    projects: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct PolicyConfig {
    backend_selection: Option<BackendSelectionPolicyConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct BackendSelectionPolicyConfig {
    #[serde(alias = "providerAgnosticWeight")]
    provider_agnostic_weight: Option<i32>,
    #[serde(alias = "automationSkillsWeight")]
    automation_skills_weight: Option<i32>,
    #[serde(alias = "openSourceWeight")]
    open_source_weight: Option<i32>,
    #[serde(alias = "lspSupportWeight")]
    lsp_support_weight: Option<i32>,
    #[serde(alias = "privacyWeight")]
    privacy_weight: Option<i32>,
    #[serde(alias = "subscriptionPenalty")]
    subscription_penalty: Option<i32>,
}

pub struct ConfigLoader;

impl ConfigLoader {
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
            apply_file(&mut cfg, file_cfg);
        }

        let user_cfg = read_file_config(&user_config_path)?;
        if let Some(file_cfg) = user_cfg.as_ref() {
            apply_file(&mut cfg, file_cfg);
        }

        let trusted_projects =
            trusted_project_set(&[global_cfg.as_ref(), user_cfg.as_ref()], &sources.cwd);

        let env_trust = sources.read_process_env && env_truthy("RUSTCODE_TRUST_PROJECT");
        let project_explicitly_trusted = sources.trust_project || env_trust;
        let project_in_trust_list = trusted_projects.contains(&normalize_path(&sources.cwd));
        let project_trusted = project_explicitly_trusted || project_in_trust_list;

        let project_cfg = read_file_config(&project_config_path)?;
        if let Some(file_cfg) = project_cfg.as_ref() {
            if !project_trusted {
                return Err(ConfigError::Trust(format!(
                    "project config {} exists but {} is not trusted; pass --trust-project-config, set RUSTCODE_TRUST_PROJECT=1, or add the path to [trust].projects",
                    project_config_path.display(),
                    sources.cwd.display()
                )));
            }

            cfg.project_config_path = Some(project_config_path.clone());
            cfg.project_config_trusted = true;
            apply_file(&mut cfg, file_cfg);
        }

        if sources.read_process_env {
            if let Ok(profile) = std::env::var("RUSTCODE_PROFILE") {
                cfg.profile = profile;
            }
            if let Ok(model) = std::env::var("RUSTCODE_MODEL") {
                cfg.model = model;
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

        if let Some(profile) = &sources.profile_override {
            cfg.profile = profile.clone();
        }
        if let Some(model) = &sources.model_override {
            cfg.model = model.clone();
        }
        if let Some(provider) = &sources.llm_provider_override {
            cfg.llm_provider = provider.clone();
        }
        if let Some(base_url) = &sources.llm_base_url_override {
            cfg.llm_base_url = Some(base_url.clone());
        }
        if let Some(api_key_env) = &sources.llm_api_key_env_override {
            cfg.llm_api_key_env = Some(api_key_env.clone());
        }

        validate(&cfg)?;
        Ok(cfg)
    }
}

fn resolve_global_config_path(sources: &ConfigSources) -> PathBuf {
    if let Some(path) = &sources.global_config_path {
        return path.clone();
    }

    if sources.read_process_env {
        if let Ok(path) = std::env::var("RUSTCODE_GLOBAL_CONFIG") {
            return PathBuf::from(path);
        }
    }

    #[cfg(unix)]
    {
        PathBuf::from("/etc/rustcode/rustcode.toml")
    }

    #[cfg(not(unix))]
    {
        PathBuf::from(".rustcode-global.toml")
    }
}

fn resolve_user_config_path(sources: &ConfigSources) -> PathBuf {
    if let Some(path) = &sources.user_config_path {
        return path.clone();
    }

    if sources.read_process_env {
        if let Ok(path) = std::env::var("RUSTCODE_USER_CONFIG") {
            return PathBuf::from(path);
        }

        if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(path).join("rustcode/rustcode.toml");
        }

        if let Ok(path) = std::env::var("HOME") {
            return PathBuf::from(path).join(".config/rustcode/rustcode.toml");
        }
    }

    PathBuf::from(".rustcode-user.toml")
}

fn resolve_project_config_path(sources: &ConfigSources) -> PathBuf {
    if let Some(path) = &sources.project_config_path {
        return path.clone();
    }

    sources.cwd.join("rustcode.toml")
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

fn apply_file(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
    if let Some(profile) = &file_cfg.profile {
        cfg.profile = profile.clone();
    }
    if let Some(model) = &file_cfg.model {
        cfg.model = model.clone();
    }
    if let Some(llm) = &file_cfg.llm {
        if let Some(provider) = &llm.provider {
            cfg.llm_provider = provider.clone();
        }
        if let Some(base_url) = &llm.base_url {
            cfg.llm_base_url = Some(base_url.clone());
        }
        if let Some(api_key_env) = &llm.api_key_env {
            cfg.llm_api_key_env = Some(api_key_env.clone());
        }
    }
    if let Some(mcp) = &file_cfg.mcp {
        if let Some(servers) = &mcp.servers {
            merge_mcp_servers(&mut cfg.mcp_servers, servers);
        }
    }
    if let Some(allow_network) = file_cfg.allow_network {
        cfg.allow_network = allow_network;
    }
    if let Some(plugins) = &file_cfg.plugins {
        merge_plugins(&mut cfg.plugins, plugins);
    }
    if let Some(env) = &file_cfg.env {
        for (key, value) in env {
            cfg.env.insert(key.clone(), value.clone());
        }
    }
    if let Some(policy) = &file_cfg.policy {
        if let Some(backend_selection) = &policy.backend_selection {
            merge_backend_selection_policy(&mut cfg.backend_selection, backend_selection);
        }
    }
}

fn merge_mcp_servers(
    current: &mut BTreeMap<String, McpServerConfig>,
    incoming: &BTreeMap<String, McpServerFileConfig>,
) {
    for (name, server) in incoming {
        let mut entry = current.remove(name).unwrap_or(McpServerConfig {
            url: None,
            oauth: McpOAuthConfig::default(),
        });
        if server.url.is_some() {
            entry.url = server.url.clone();
        }
        let oauth = match &server.oauth {
            None => entry.oauth.clone(),
            Some(McpOAuthFileConfig::Enabled(enabled)) => McpOAuthConfig {
                enabled: *enabled,
                client_id: entry.oauth.client_id.clone(),
                client_secret_env: entry.oauth.client_secret_env.clone(),
            },
            Some(McpOAuthFileConfig::Settings(settings)) => McpOAuthConfig {
                enabled: settings.enabled.unwrap_or(true),
                client_id: settings.client_id.clone().or(entry.oauth.client_id),
                client_secret_env: settings
                    .client_secret_env
                    .clone()
                    .or(entry.oauth.client_secret_env),
            },
        };
        entry.oauth = oauth;
        current.insert(name.clone(), entry);
    }
}

fn merge_backend_selection_policy(
    current: &mut BackendSelectionPolicy,
    incoming: &BackendSelectionPolicyConfig,
) {
    if let Some(value) = incoming.provider_agnostic_weight {
        current.provider_agnostic_weight = value;
    }
    if let Some(value) = incoming.automation_skills_weight {
        current.automation_skills_weight = value;
    }
    if let Some(value) = incoming.open_source_weight {
        current.open_source_weight = value;
    }
    if let Some(value) = incoming.lsp_support_weight {
        current.lsp_support_weight = value;
    }
    if let Some(value) = incoming.privacy_weight {
        current.privacy_weight = value;
    }
    if let Some(value) = incoming.subscription_penalty {
        current.subscription_penalty = value;
    }
}

fn merge_plugins(current: &mut Vec<String>, incoming: &[String]) {
    for plugin in incoming {
        if !current.iter().any(|item| item == plugin) {
            current.push(plugin.clone());
        }
    }
}

fn trusted_project_set(configs: &[Option<&FileConfig>], cwd: &Path) -> BTreeSet<PathBuf> {
    let mut trusted = BTreeSet::new();

    for cfg in configs {
        if let Some(cfg) = cfg {
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

fn validate(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
    if cfg.profile.trim().is_empty() {
        return Err(ConfigError::Validation(
            "profile must not be empty".to_string(),
        ));
    }
    if cfg.model.trim().is_empty() {
        return Err(ConfigError::Validation(
            "model must not be empty".to_string(),
        ));
    }
    if cfg.llm_provider.trim().is_empty() {
        return Err(ConfigError::Validation(
            "llm.provider must not be empty".to_string(),
        ));
    }
    if let Some(api_key_env) = cfg.llm_api_key_env.as_ref() {
        if api_key_env.trim().is_empty() {
            return Err(ConfigError::Validation(
                "llm.api_key_env must not be empty".to_string(),
            ));
        }
    }
    for (name, server) in &cfg.mcp_servers {
        if name.trim().is_empty() {
            return Err(ConfigError::Validation(
                "mcp.servers entries must have non-empty names".to_string(),
            ));
        }
        if let Some(url) = server.url.as_ref() {
            if url.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "mcp.servers.{name}.url must not be empty"
                )));
            }
        }
        if let Some(client_id) = server.oauth.client_id.as_ref() {
            if client_id.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "mcp.servers.{name}.oauth.client_id must not be empty"
                )));
            }
        }
        if let Some(secret_env) = server.oauth.client_secret_env.as_ref() {
            if secret_env.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "mcp.servers.{name}.oauth.client_secret_env must not be empty"
                )));
            }
        }
    }
    validate_non_negative(
        "policy.backend_selection.provider_agnostic_weight",
        cfg.backend_selection.provider_agnostic_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.automation_skills_weight",
        cfg.backend_selection.automation_skills_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.open_source_weight",
        cfg.backend_selection.open_source_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.lsp_support_weight",
        cfg.backend_selection.lsp_support_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.privacy_weight",
        cfg.backend_selection.privacy_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.subscription_penalty",
        cfg.backend_selection.subscription_penalty,
    )?;
    Ok(())
}

fn validate_non_negative(field: &str, value: i32) -> Result<(), ConfigError> {
    if value < 0 {
        return Err(ConfigError::Validation(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn defaults_validate() {
        let temp_root = make_temp_dir("defaults");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert!(!cfg.profile.is_empty());
        assert!(!cfg.model.is_empty());
    }

    #[test]
    fn applies_layered_precedence_and_merges_env_and_plugins() {
        let temp_root = make_temp_dir("precedence");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let global = temp_root.join("global.toml");
        let user = temp_root.join("user.toml");
        let project = cwd.join("rustcode.toml");

        write_config(
            &global,
            r#"
profile = "global"
model = "global-model"
allow_network = false
plugins = ["g1"]

[env]
A = "global-a"
B = "global-b"
"#,
        );

        write_config(
            &user,
            &format!(
                r#"
profile = "user"
plugins = ["u1", "g1"]

[env]
B = "user-b"
C = "user-c"

[trust]
projects = ["{}"]
"#,
                cwd.display()
            ),
        );

        write_config(
            &project,
            r#"
model = "project-model"
allow_network = true
plugins = ["p1"]

[env]
A = "project-a"
"#,
        );

        let mut sources = ConfigSources::new(cwd.clone());
        sources.read_process_env = false;
        sources.global_config_path = Some(global);
        sources.user_config_path = Some(user);
        sources.project_config_path = Some(project.clone());

        let cfg = ConfigLoader::load(&sources).expect("config should load");

        assert_eq!(cfg.profile, "user");
        assert_eq!(cfg.model, "project-model");
        assert!(cfg.allow_network);
        assert_eq!(cfg.plugins, vec!["g1", "u1", "p1"]);
        assert_eq!(cfg.env.get("A").expect("A must exist"), "project-a");
        assert_eq!(cfg.env.get("B").expect("B must exist"), "user-b");
        assert_eq!(cfg.env.get("C").expect("C must exist"), "user-c");
        assert_eq!(cfg.project_config_path, Some(project));
        assert!(cfg.project_config_trusted);
    }

    #[test]
    fn rejects_untrusted_project_config() {
        let temp_root = make_temp_dir("untrusted");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let user = temp_root.join("user.toml");
        let project = cwd.join("rustcode.toml");

        write_config(&user, "profile = \"user\"\n");
        write_config(&project, "model = \"project\"\n");

        let mut sources = ConfigSources::new(cwd.clone());
        sources.read_process_env = false;
        sources.user_config_path = Some(user);
        sources.project_config_path = Some(project);

        let err = ConfigLoader::load(&sources).expect_err("must reject untrusted config");
        match err {
            ConfigError::Trust(message) => {
                assert!(message.contains("not trusted"));
                assert!(message.contains(&cwd.display().to_string()));
            }
            _ => panic!("expected trust error"),
        }
    }

    #[test]
    fn cli_overrides_are_highest_precedence() {
        let temp_root = make_temp_dir("cli-overrides");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let user = temp_root.join("user.toml");
        let project = cwd.join("rustcode.toml");

        write_config(
            &user,
            &format!(
                r#"
profile = "user"
model = "user-model"

[trust]
projects = ["{}"]
"#,
                cwd.display()
            ),
        );
        write_config(
            &project,
            "profile = \"project\"\nmodel = \"project-model\"\n",
        );

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;
        sources.user_config_path = Some(user);
        sources.project_config_path = Some(project);
        sources.profile_override = Some("cli-profile".to_string());
        sources.model_override = Some("cli-model".to_string());

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert_eq!(cfg.profile, "cli-profile");
        assert_eq!(cfg.model, "cli-model");
    }

    #[test]
    fn llm_config_layers_and_cli_override_apply() {
        let temp_root = make_temp_dir("llm-config");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let user = temp_root.join("user.toml");
        let project = cwd.join("rustcode.toml");

        write_config(
            &user,
            &format!(
                r#"
[llm]
provider = "openrouter"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[trust]
projects = ["{}"]
"#,
                cwd.display()
            ),
        );
        write_config(
            &project,
            r#"
[llm]
provider = "anthropic"
baseURL = "https://api.anthropic.com"
apiKeyEnv = "ANTHROPIC_API_KEY"
"#,
        );

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;
        sources.user_config_path = Some(user);
        sources.project_config_path = Some(project);
        sources.llm_provider_override = Some("ollama".to_string());

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert_eq!(cfg.llm_provider, "ollama");
        assert_eq!(
            cfg.llm_base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert_eq!(cfg.llm_api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn mcp_server_config_layers_and_merges() {
        let temp_root = make_temp_dir("mcp-config");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let global = temp_root.join("global.toml");
        let user = temp_root.join("user.toml");
        let project = cwd.join("rustcode.toml");

        write_config(
            &global,
            r#"
[mcp.servers.github]
url = "https://global.example.com/mcp"
oauth = true
"#,
        );
        write_config(
            &user,
            &format!(
                r#"
[mcp.servers.github]
url = "https://user.example.com/mcp"
oauth = {{ enabled = true, client_id = "user-client" }}

[trust]
projects = ["{}"]
"#,
                cwd.display()
            ),
        );
        write_config(
            &project,
            r#"
[mcp.servers.github]
oauth = { enabled = false }

[mcp.servers.linear]
url = "https://linear.example.com/mcp"
"#,
        );

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;
        sources.global_config_path = Some(global);
        sources.user_config_path = Some(user);
        sources.project_config_path = Some(project);
        sources.trust_project = true;

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert_eq!(cfg.mcp_servers.len(), 2);

        let github = cfg
            .mcp_servers
            .get("github")
            .expect("github mcp config should exist");
        assert_eq!(github.url.as_deref(), Some("https://user.example.com/mcp"));
        assert!(!github.oauth.enabled);
        assert_eq!(github.oauth.client_id.as_deref(), Some("user-client"));

        let linear = cfg
            .mcp_servers
            .get("linear")
            .expect("linear mcp config should exist");
        assert_eq!(
            linear.url.as_deref(),
            Some("https://linear.example.com/mcp")
        );
        assert!(linear.oauth.enabled);
    }

    #[test]
    fn mcp_server_config_rejects_empty_url() {
        let temp_root = make_temp_dir("mcp-config-invalid");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");
        let global = temp_root.join("global.toml");

        write_config(
            &global,
            r#"
[mcp.servers.github]
url = "   "
"#,
        );

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;
        sources.global_config_path = Some(global);

        let err = ConfigLoader::load(&sources).expect_err("must reject empty mcp url");
        match err {
            ConfigError::Validation(message) => {
                assert!(message.contains("mcp.servers.github.url"));
            }
            _ => panic!("expected validation error"),
        }
    }

    #[test]
    fn backend_selection_policy_layers_and_validates() {
        let temp_root = make_temp_dir("backend-selection-policy");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");

        let global = temp_root.join("global.toml");
        let project = cwd.join("rustcode.toml");

        write_config(
            &global,
            r#"
[policy.backend_selection]
provider_agnostic_weight = 4
automation_skills_weight = 2
"#,
        );

        write_config(
            &project,
            &format!(
                r#"
[policy.backend_selection]
privacy_weight = 5
subscription_penalty = 3

[trust]
projects = ["{}"]
"#,
                cwd.display()
            ),
        );

        let mut sources = ConfigSources::new(cwd.clone());
        sources.read_process_env = false;
        sources.global_config_path = Some(global);
        sources.project_config_path = Some(project);
        sources.trust_project = true;

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert_eq!(cfg.backend_selection.provider_agnostic_weight, 4);
        assert_eq!(cfg.backend_selection.automation_skills_weight, 2);
        assert_eq!(cfg.backend_selection.open_source_weight, 1);
        assert_eq!(cfg.backend_selection.lsp_support_weight, 1);
        assert_eq!(cfg.backend_selection.privacy_weight, 5);
        assert_eq!(cfg.backend_selection.subscription_penalty, 3);
    }

    #[test]
    fn backend_selection_policy_rejects_negative_weight() {
        let temp_root = make_temp_dir("backend-selection-policy-negative");
        let cwd = temp_root.join("project");
        fs::create_dir_all(&cwd).expect("must create cwd");
        let global = temp_root.join("global.toml");

        write_config(
            &global,
            r#"
[policy.backend_selection]
privacy_weight = -1
"#,
        );

        let mut sources = ConfigSources::new(cwd);
        sources.read_process_env = false;
        sources.global_config_path = Some(global);

        let err = ConfigLoader::load(&sources).expect_err("must reject negative weight");
        match err {
            ConfigError::Validation(message) => {
                assert!(message.contains("privacy_weight"));
                assert!(message.contains("must not be negative"));
            }
            _ => panic!("expected validation error"),
        }
    }

    fn write_config(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("must create parent");
        }
        fs::write(path, contents).expect("must write file");
    }

    fn make_temp_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time must be monotonic")
            .as_nanos();
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("rustcode-config-{name}-{pid}-{now}"));
        fs::create_dir_all(&dir).expect("must create temp dir");
        dir
    }
}
