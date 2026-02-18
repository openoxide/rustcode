use std::path::PathBuf;

use rustcode_core::error::ConfigError;

use crate::ConfigSources;

pub(crate) fn parse_boolish_env(key: &str, value: &str) -> Result<bool, ConfigError> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Validation(format!(
            "{key} must be a boolean (accepted: true/false/1/0/on/off/yes/no)"
        ))),
    }
}

pub(crate) fn resolve_global_config_path(sources: &ConfigSources) -> PathBuf {
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

pub(crate) fn resolve_user_config_path(sources: &ConfigSources) -> PathBuf {
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

pub(crate) fn resolve_project_config_path(sources: &ConfigSources) -> PathBuf {
    if let Some(path) = &sources.project_config_path {
        return path.clone();
    }

    sources.cwd.join("rustcode.toml")
}
