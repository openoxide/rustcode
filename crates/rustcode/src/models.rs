use std::collections::BTreeMap;

use crate::auth::{load_models_index, ModelsProvider};
use crate::cli::Cli;
use crate::utils::{load_effective_config, write_stdout_line};
use anyhow::Result;
use rustcode_auth::{AuthStore, StoredCredential};
use rustcode_llm::{builtin_provider_ids, diagnose_provider, ApiKeySource, ProviderProtocolName};

pub fn handle_models_command(
    provider_id: Option<String>,
    json_output: bool,
    cli: &Cli,
) -> Result<()> {
    let config = load_effective_config(cli)?;
    let (index, index_error) = load_models_index_with_warning();

    if let Some(provider_id) = provider_id {
        let diag = diagnose_provider(&config, Some(&provider_id))?;
        let provider_meta = index.get(&provider_id);
        let models = provider_models(&provider_id, provider_meta);
        let warning = if index_error.is_some() {
            Some("models index unavailable; diagnostics only".to_string())
        } else if provider_meta.is_none() {
            Some("provider not found in models index; diagnostics only".to_string())
        } else {
            None
        };

        if json_output {
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "models",
                "provider": {
                    "id": provider_id,
                    "protocol": render_protocol(&diag.protocol),
                    "base_url": diag.base_url,
                    "endpoint": diag.endpoint,
                    "requires_api_key": diag.requires_api_key,
                    "api_key_source": render_api_key_source(&diag.api_key_source),
                    "missing": diag.missing,
                    "policy_score": diag.policy_score,
                    "policy_selected": diag.policy_selected,
                    "policy_available": diag.policy_available,
                    "models": models,
                    "warning": warning,
                },
            });
            write_stdout_line(&serde_json::to_string(&payload)?)?;
        } else {
            write_stdout_line(&format!("provider={provider_id}"))?;
            write_stdout_line(&format!("protocol={}", render_protocol(&diag.protocol)))?;
            write_stdout_line(&format!(
                "base_url={}",
                diag.base_url.as_deref().unwrap_or("default")
            ))?;
            write_stdout_line(&format!(
                "api_key_source={}",
                render_api_key_source(&diag.api_key_source)
            ))?;
            if !diag.missing.is_empty() {
                write_stdout_line(&format!("missing={}", diag.missing.join(",")))?;
            }
            if let Some(warning) = warning {
                write_stdout_line(&format!("warning={warning}"))?;
            }
            for model in provider_models(&provider_id, provider_meta) {
                write_stdout_line(&model)?;
            }
        }
    } else {
        let mut provider_ids = if index_error.is_some() {
            builtin_provider_ids()
                .iter()
                .map(|id| (*id).to_string())
                .collect::<Vec<_>>()
        } else {
            index.keys().cloned().collect::<Vec<_>>()
        };
        provider_ids.sort();

        if json_output {
            let mut rows = Vec::new();
            for id in provider_ids {
                if let Ok(diag) = diagnose_provider(&config, Some(&id)) {
                    rows.push(serde_json::json!({
                        "id": id,
                        "protocol": render_protocol(&diag.protocol),
                        "api_key_source": render_api_key_source(&diag.api_key_source),
                        "missing": diag.missing,
                        "policy_score": diag.policy_score,
                        "policy_selected": diag.policy_selected,
                        "policy_available": diag.policy_available,
                    }));
                }
            }
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "models",
                "warning": if index_error.is_some() {
                    Some("models index unavailable; showing builtin presets only")
                } else {
                    None
                },
                "providers": rows,
            });
            write_stdout_line(&serde_json::to_string(&payload)?)?;
        } else {
            write_stdout_line(&format!("providers={}", provider_ids.len()))?;
            if index_error.is_some() {
                write_stdout_line(
                    "warning=models index unavailable; showing builtin presets only",
                )?;
            }
            for id in provider_ids {
                if let Ok(diag) = diagnose_provider(&config, Some(&id)) {
                    write_stdout_line(&format!(
                        "provider={id} protocol={} api_key_source={} missing={} policy_score={} policy_selected={} policy_available={}",
                        render_protocol(&diag.protocol),
                        render_api_key_source(&diag.api_key_source),
                        diag.missing.join(","),
                        diag.policy_score
                            .map_or_else(|| "null".to_string(), |value| value.to_string()),
                        diag.policy_selected,
                        diag.policy_available
                            .map_or_else(|| "null".to_string(), |value| value.to_string())
                    ))?;
                }
            }
        }
    }
    Ok(())
}

pub fn render_protocol(protocol: &ProviderProtocolName) -> &'static str {
    match protocol {
        ProviderProtocolName::OpenAiCompatible => "openai",
        ProviderProtocolName::AnthropicMessages => "anthropic",
        ProviderProtocolName::GoogleGenerativeAi => "google",
        ProviderProtocolName::VercelAiGateway => "vercel",
        ProviderProtocolName::Null => "null",
    }
}

pub fn render_api_key_source(source: &ApiKeySource) -> String {
    match source {
        ApiKeySource::ConfigEnvMap { var_name } => format!("config_env({var_name})"),
        ApiKeySource::ProcessEnv { var_name } => format!("process_env({var_name})"),
        ApiKeySource::AuthStore => "auth_store".to_string(),
        ApiKeySource::None => "none".to_string(),
    }
}

fn load_models_index_with_warning() -> (BTreeMap<String, ModelsProvider>, Option<String>) {
    match load_models_index() {
        Ok(index) => (index, None),
        Err(err) => (BTreeMap::new(), Some(err.to_string())),
    }
}

fn provider_models(provider_id: &str, provider: Option<&ModelsProvider>) -> Vec<String> {
    let mut models = provider
        .map(|meta| {
            meta.models
                .keys()
                .map(|model| format!("{provider_id}/{model}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if provider_id.eq_ignore_ascii_case("openai") && openai_oauth_mode() {
        models.retain(|entry| {
            let Some((_, model_id)) = entry.split_once('/') else {
                return true;
            };
            openai_oauth_model_allowed(model_id)
        });
    }
    models.sort();
    models
}

fn openai_oauth_mode() -> bool {
    let store = AuthStore::open_default();
    matches!(
        store.get("openai").ok().flatten(),
        Some(StoredCredential::OAuth { .. })
    )
}

fn openai_oauth_model_allowed(model_id: &str) -> bool {
    if model_id.to_ascii_lowercase().contains("codex") {
        return true;
    }
    matches!(
        model_id,
        "gpt-5.1-codex-max"
            | "gpt-5.1-codex-mini"
            | "gpt-5.2"
            | "gpt-5.2-codex"
            | "gpt-5.3-codex"
            | "gpt-5.1-codex"
    )
}
