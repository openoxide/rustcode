use std::collections::BTreeMap;

use rustcode_core::config::{
    BackendSelectionPolicy, McpOAuthConfig, McpServerConfig, ResolvedConfig,
};
use rustcode_core::permissions::PermissionRule;

use crate::{
    BackendSelectionPolicyConfig, FileConfig, LlmConfig, McpOAuthFileConfig,
    McpServerFileConfig,
};

/// Apply a parsed file config layer onto the resolved config.
pub(crate) fn apply_file(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
    apply_basic_fields(cfg, file_cfg);
    apply_providers(cfg, file_cfg);
    apply_mcp(cfg, file_cfg);
    apply_misc(cfg, file_cfg);
}

fn apply_basic_fields(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
    if let Some(profile) = &file_cfg.profile {
        cfg.profile.clone_from(profile);
    }
    if let Some(model) = &file_cfg.model {
        cfg.model.clone_from(model);
    }
    if let Some(llm) = &file_cfg.llm {
        apply_llm(cfg, llm);
    }
}

fn apply_llm(cfg: &mut ResolvedConfig, llm: &LlmConfig) {
    if let Some(provider) = &llm.provider {
        cfg.llm_provider.clone_from(provider);
    }
    if let Some(base_url) = &llm.base_url {
        cfg.llm_base_url = Some(base_url.clone());
    }
    if let Some(api_key_env) = &llm.api_key_env {
        cfg.llm_api_key_env = Some(api_key_env.clone());
    }
}

fn apply_providers(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
    if let Some(enabled) = &file_cfg.enabled_providers {
        cfg.enabled_providers = Some(
            enabled
                .iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
        );
    }
    if let Some(disabled) = &file_cfg.disabled_providers {
        cfg.disabled_providers = disabled
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();
    }
}

fn apply_mcp(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
    if let Some(mcp) = &file_cfg.mcp {
        if let Some(servers) = &mcp.servers {
            merge_mcp_servers(&mut cfg.mcp_servers, servers);
        }
    }
}

fn apply_misc(cfg: &mut ResolvedConfig, file_cfg: &FileConfig) {
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
    if let Some(rules) = &file_cfg.permissions {
        for rule in rules {
            cfg.permission_rules.push(PermissionRule {
                permission: rule.permission.trim().to_string(),
                action: rule.action,
                pattern: rule.pattern.trim().to_string(),
            });
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
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            oauth: McpOAuthConfig::default(),
        });
        if server.url.is_some() {
            entry.url.clone_from(&server.url);
        }
        if server.command.is_some() {
            entry.command.clone_from(&server.command);
        }
        if !server.args.is_empty() {
            entry.args.clone_from(&server.args);
        }
        if !server.env.is_empty() {
            entry.env.clone_from(&server.env);
        }
        entry.oauth = resolve_mcp_oauth(&server.oauth, &entry.oauth);
        current.insert(name.clone(), entry);
    }
}

fn resolve_mcp_oauth(
    incoming: &Option<McpOAuthFileConfig>,
    current: &McpOAuthConfig,
) -> McpOAuthConfig {
    match incoming {
        None => current.clone(),
        Some(McpOAuthFileConfig::Enabled(enabled)) => McpOAuthConfig {
            enabled: *enabled,
            client_id: current.client_id.clone(),
            client_secret_env: current.client_secret_env.clone(),
        },
        Some(McpOAuthFileConfig::Settings(settings)) => McpOAuthConfig {
            enabled: settings.enabled.unwrap_or(true),
            client_id: settings.client_id.clone().or(current.client_id.clone()),
            client_secret_env: settings
                .client_secret_env
                .clone()
                .or(current.client_secret_env.clone()),
        },
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
