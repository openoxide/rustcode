use globset::Glob;
use rustcode_core::config::{BackendSelectionPolicy, ResolvedConfig};
use rustcode_core::error::ConfigError;

/// Validate the fully resolved configuration.
///
/// # Errors
/// Returns `ConfigError::Validation` when any config invariant is violated.
pub(crate) fn validate(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
    validate_basic_fields(cfg)?;
    validate_providers(cfg)?;
    validate_mcp_servers(cfg)?;
    validate_backend_selection(&cfg.backend_selection)?;
    validate_permissions(cfg)?;
    Ok(())
}

fn validate_basic_fields(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
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
    Ok(())
}

fn validate_providers(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
    if let Some(enabled) = cfg.enabled_providers.as_ref() {
        for provider in enabled {
            validate_provider_id("enabled_providers", provider)?;
        }
    }
    for provider in &cfg.disabled_providers {
        validate_provider_id("disabled_providers", provider)?;
    }
    Ok(())
}

fn validate_mcp_servers(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
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
        if let Some(command) = server.command.as_ref() {
            if command.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "mcp.servers.{name}.command must not be empty"
                )));
            }
        }
        if server.url.is_none() && server.command.is_none() {
            return Err(ConfigError::Validation(format!(
                "mcp.servers.{name} must set either url or command"
            )));
        }
        if server.url.is_some() && server.command.is_some() {
            return Err(ConfigError::Validation(format!(
                "mcp.servers.{name} cannot set both url and command"
            )));
        }
        validate_mcp_server_details(name, server)?;
    }
    Ok(())
}

fn validate_mcp_server_details(
    name: &str,
    server: &rustcode_core::config::McpServerConfig,
) -> Result<(), ConfigError> {
    for (idx, arg) in server.args.iter().enumerate() {
        if arg.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "mcp.servers.{name}.args[{idx}] must not be empty"
            )));
        }
    }
    for (key, value) in &server.env {
        if key.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "mcp.servers.{name}.env keys must not be empty"
            )));
        }
        if value.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "mcp.servers.{name}.env.{key} must not be empty"
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
    Ok(())
}

fn validate_backend_selection(policy: &BackendSelectionPolicy) -> Result<(), ConfigError> {
    validate_non_negative(
        "policy.backend_selection.provider_agnostic_weight",
        policy.provider_agnostic_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.automation_skills_weight",
        policy.automation_skills_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.open_source_weight",
        policy.open_source_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.lsp_support_weight",
        policy.lsp_support_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.privacy_weight",
        policy.privacy_weight,
    )?;
    validate_non_negative(
        "policy.backend_selection.subscription_penalty",
        policy.subscription_penalty,
    )?;
    Ok(())
}

fn validate_permissions(cfg: &ResolvedConfig) -> Result<(), ConfigError> {
    for (idx, rule) in cfg.permission_rules.iter().enumerate() {
        if rule.permission.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "permissions[{idx}].permission must not be empty"
            )));
        }
        if rule.pattern.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "permissions[{idx}].pattern must not be empty"
            )));
        }
        Glob::new(&rule.pattern).map_err(|err| {
            ConfigError::Validation(format!(
                "permissions[{idx}].pattern is not a valid glob: {err}"
            ))
        })?;
    }
    Ok(())
}

pub(crate) fn validate_non_negative(field: &str, value: i32) -> Result<(), ConfigError> {
    if value < 0 {
        return Err(ConfigError::Validation(format!(
            "{field} must not be negative"
        )));
    }
    Ok(())
}

pub(crate) fn validate_provider_id(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::Validation(format!(
            "{field} entries must not be empty"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(ConfigError::Validation(format!(
            "{field} provider id must match [a-z0-9-]+: {value}"
        )));
    }
    Ok(())
}
