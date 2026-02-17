use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rustcode_core::config::ResolvedConfig;
use rustcode_core::error::ConfigError;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct ConfigSources {
    pub cwd: PathBuf,
    pub profile_override: Option<String>,
    pub model_override: Option<String>,
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
    allow_network: Option<bool>,
    plugins: Option<Vec<String>>,
    env: Option<BTreeMap<String, String>>,
    trust: Option<TrustConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct TrustConfig {
    projects: Option<Vec<String>>,
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
        }

        if let Some(profile) = &sources.profile_override {
            cfg.profile = profile.clone();
        }
        if let Some(model) = &sources.model_override {
            cfg.model = model.clone();
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
