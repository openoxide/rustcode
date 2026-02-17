use std::collections::BTreeMap;
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
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    profile: Option<String>,
    model: Option<String>,
    allow_network: Option<bool>,
    plugins: Option<Vec<String>>,
    env: Option<BTreeMap<String, String>>,
}

pub struct ConfigLoader;

impl ConfigLoader {
    pub fn load(sources: &ConfigSources) -> Result<ResolvedConfig, ConfigError> {
        let mut cfg = ResolvedConfig {
            workspace_root: sources.cwd.clone(),
            ..ResolvedConfig::default()
        };

        let file_cfg = read_file_config(&sources.cwd.join("rustcode.toml"))?;
        if let Some(file_cfg) = file_cfg {
            apply_file(&mut cfg, file_cfg);
        }

        if let Ok(profile) = std::env::var("RUSTCODE_PROFILE") {
            cfg.profile = profile;
        }
        if let Ok(model) = std::env::var("RUSTCODE_MODEL") {
            cfg.model = model;
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

fn apply_file(cfg: &mut ResolvedConfig, file_cfg: FileConfig) {
    if let Some(profile) = file_cfg.profile {
        cfg.profile = profile;
    }
    if let Some(model) = file_cfg.model {
        cfg.model = model;
    }
    if let Some(allow_network) = file_cfg.allow_network {
        cfg.allow_network = allow_network;
    }
    if let Some(plugins) = file_cfg.plugins {
        cfg.plugins = plugins;
    }
    if let Some(env) = file_cfg.env {
        cfg.env = env;
    }
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
    use super::*;

    #[test]
    fn defaults_validate() {
        let sources = ConfigSources {
            cwd: std::env::temp_dir(),
            profile_override: None,
            model_override: None,
        };

        let cfg = ConfigLoader::load(&sources).expect("config should load");
        assert!(!cfg.profile.is_empty());
        assert!(!cfg.model.is_empty());
    }
}
