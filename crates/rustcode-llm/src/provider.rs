use crate::endpoints::normalize_endpoint_for_provider;
use crate::provider_presets::{collect_provider_api_key_envs, provider_preset};
use crate::types::LlmError;
use rustcode_auth::{AuthStore, StoredCredential};
use rustcode_core::config::ResolvedConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderProtocol {
    Null,
    OpenAiCompatible,
    AnthropicMessages,
    VercelAiGateway,
    GoogleGenerativeAi,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProvider {
    pub(crate) provider_id: String,
    pub(crate) protocol: ProviderProtocol,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) requires_api_key: bool,
    pub(crate) api_key_env_candidates: Vec<String>,
    pub(crate) api_key_source: ApiKeySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeySource {
    None,
    ConfigEnvMap { var_name: String },
    ProcessEnv { var_name: String },
    AuthStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderProtocolName {
    Null,
    OpenAiCompatible,
    AnthropicMessages,
    VercelAiGateway,
    GoogleGenerativeAi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiagnostics {
    pub provider_id: String,
    pub protocol: ProviderProtocolName,
    pub base_url: Option<String>,
    pub endpoint: Option<String>,
    pub requires_api_key: bool,
    pub api_key_source: ApiKeySource,
    pub api_key_env_candidates: Vec<String>,
    pub missing: Vec<String>,
    pub policy_score: Option<i32>,
    pub policy_selected: bool,
    pub policy_available: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct BackendPolicyCandidate {
    provider_id: &'static str,
    provider_agnostic: bool,
    automation_skills: bool,
    open_source: bool,
    lsp_support: bool,
    privacy: bool,
    subscription_required: bool,
}

#[derive(Debug, Clone, Copy)]
struct BackendPolicyEvaluation {
    score: i32,
    available: bool,
}

const BACKEND_POLICY_CANDIDATES: &[BackendPolicyCandidate] = &[
    BackendPolicyCandidate {
        provider_id: "openrouter",
        provider_agnostic: true,
        automation_skills: false,
        open_source: true,
        lsp_support: true,
        privacy: true,
        subscription_required: false,
    },
    BackendPolicyCandidate {
        provider_id: "openai",
        provider_agnostic: false,
        automation_skills: true,
        open_source: false,
        lsp_support: false,
        privacy: false,
        subscription_required: true,
    },
];

pub fn diagnose_provider(
    config: &ResolvedConfig,
    provider_id: Option<&str>,
) -> Result<ProviderDiagnostics, LlmError> {
    let policy_selected_provider = select_provider_by_policy(config).map(|value| value.to_string());
    let policy_eval_for_input =
        provider_id.and_then(|value| evaluate_backend_policy(config, value));

    let mut effective = config.clone();
    if let Some(provider) = provider_id {
        if !provider.eq_ignore_ascii_case(&effective.llm_provider) {
            effective.llm_base_url = None;
            effective.llm_api_key_env = None;
        }
        effective.llm_provider = provider.to_string();
        effective.model = format!("{provider}/diagnostic-model");
    }

    let resolved = resolve_provider(&effective)?;
    let endpoint = resolved.base_url.as_ref().map(|base| {
        normalize_endpoint_for_provider(resolved.protocol, base, &resolved.provider_id)
    });
    let mut missing = Vec::new();
    if resolved.base_url.is_none() && !matches!(resolved.protocol, ProviderProtocol::Null) {
        missing.push("base_url".to_string());
    }
    if resolved.requires_api_key && resolved.api_key.is_none() {
        missing.push("api_key".to_string());
    }
    let policy_eval =
        policy_eval_for_input.or_else(|| evaluate_backend_policy(config, &resolved.provider_id));
    let policy_selected = policy_selected_provider
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(&resolved.provider_id));

    Ok(ProviderDiagnostics {
        provider_id: resolved.provider_id,
        protocol: protocol_name(resolved.protocol),
        base_url: resolved.base_url,
        endpoint,
        requires_api_key: resolved.requires_api_key,
        api_key_source: resolved.api_key_source,
        api_key_env_candidates: resolved.api_key_env_candidates,
        missing,
        policy_score: policy_eval.map(|value| value.score),
        policy_selected,
        policy_available: policy_eval.map(|value| value.available),
    })
}

pub fn builtin_provider_ids() -> &'static [&'static str] {
    &[
        "null",
        "openai",
        "openrouter",
        "anthropic",
        "google",
        "vercel",
        "github-copilot",
        "github-copilot-enterprise",
        "v0",
        "ollama",
        "groq",
        "xai",
        "mistral",
        "togetherai",
        "perplexity",
        "deepinfra",
        "cerebras",
        "azure",
        "azure-cognitive-services",
        "cloudflare-workers-ai",
        "cloudflare-ai-gateway",
        "gitlab",
        "opencode",
        "amazon-bedrock",
        "google-vertex",
        "google-vertex-anthropic",
        "sap-ai-core",
        "zenmux",
        "fetch",
    ]
}

pub(crate) fn resolve_provider(config: &ResolvedConfig) -> Result<ResolvedProvider, LlmError> {
    let (model_provider, _) = parse_model_prefix(&config.model).unwrap_or(("", ""));
    let provider_id: String =
        if !config.llm_provider.trim().is_empty() && config.llm_provider != "null" {
            config.llm_provider.clone()
        } else if !model_provider.is_empty() {
            model_provider.to_string()
        } else if config.allow_network {
            select_provider_by_policy(config)
                .unwrap_or("null")
                .to_string()
        } else {
            "null".to_string()
        };

    if !config.provider_allowed(&provider_id) && provider_id != "null" {
        return Err(LlmError::Config(format!(
            "provider '{provider_id}' is disabled by config"
        )));
    }

    let preset = provider_preset(&provider_id);
    let mut base_url = config.llm_base_url.clone();
    if base_url.is_none() {
        base_url = resolve_provider_base_url_from_env(config, &provider_id);
    }
    let base_url = base_url.or_else(|| preset.default_base_url.clone());
    let api_key_env_candidates = collect_provider_api_key_envs(&provider_id, &preset);
    let (api_key, api_key_source) = resolve_api_key(config, &api_key_env_candidates, &provider_id);
    let requires_api_key = preset.requires_api_key || !api_key_env_candidates.is_empty();

    Ok(ResolvedProvider {
        provider_id,
        protocol: preset.protocol,
        base_url,
        api_key,
        requires_api_key,
        api_key_env_candidates,
        api_key_source,
    })
}

fn resolve_provider_base_url_from_env(
    config: &ResolvedConfig,
    provider_id: &str,
) -> Option<String> {
    match provider_id {
        "github-copilot" => {
            read_config_or_env(config, "GITHUB_COPILOT_BASE_URL").map(|pair| pair.0)
        }
        "github-copilot-enterprise" => {
            if let Some((value, _)) =
                read_config_or_env(config, "GITHUB_COPILOT_ENTERPRISE_BASE_URL")
            {
                return Some(value);
            }
            if let Some((domain, _)) =
                read_config_or_env(config, "GITHUB_COPILOT_ENTERPRISE_DOMAIN")
            {
                return Some(derive_copilot_enterprise_base_url(domain.trim()));
            }

            let store = AuthStore::open_default();
            if let Some(credential) = store.get(provider_id).ok().flatten() {
                let domain = match credential {
                    StoredCredential::ApiKey { domain, .. } => domain,
                    StoredCredential::OAuth { domain, .. } => domain,
                };
                if let Some(domain) = domain.as_deref().filter(|value| !value.trim().is_empty()) {
                    return Some(derive_copilot_enterprise_base_url(domain));
                }
            }
            None
        }
        _ => None,
    }
}

pub fn derive_copilot_enterprise_base_url(domain: &str) -> String {
    if domain.is_empty() {
        return "".to_string();
    }

    if domain.contains("://") {
        return domain.trim_end_matches('/').to_string();
    }

    let domain = domain.trim_end_matches('/');
    if domain.starts_with("copilot-api.") {
        format!("https://{domain}")
    } else {
        format!("https://copilot-api.{domain}")
    }
}

fn resolve_api_key(
    config: &ResolvedConfig,
    env_candidates: &[String],
    provider_id: &str,
) -> (Option<String>, ApiKeySource) {
    if let Some(explicit_env) = config.llm_api_key_env.as_ref() {
        if let Some((value, source)) = read_config_or_env(config, explicit_env) {
            return (Some(value), source);
        }
    }

    for env_name in env_candidates {
        if let Some((value, source)) = read_config_or_env(config, env_name) {
            return (Some(value), source);
        }
    }

    let store = AuthStore::open_default();
    if let Some(credential) = store.get(provider_id).ok().flatten() {
        match credential {
            StoredCredential::ApiKey { key, .. } => {
                if !key.trim().is_empty() {
                    return (Some(key), ApiKeySource::AuthStore);
                }
            }
            StoredCredential::OAuth { access_token, .. } => {
                if !access_token.trim().is_empty() {
                    return (Some(access_token), ApiKeySource::AuthStore);
                }
            }
        }
    }

    (None, ApiKeySource::None)
}

fn select_provider_by_policy(config: &ResolvedConfig) -> Option<&'static str> {
    BACKEND_POLICY_CANDIDATES
        .iter()
        .filter_map(|candidate| {
            if !config.provider_allowed(candidate.provider_id) {
                return None;
            }
            evaluate_backend_policy(config, candidate.provider_id).map(|evaluation| {
                (
                    candidate.provider_id,
                    evaluation.score,
                    evaluation.available,
                )
            })
        })
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| left.2.cmp(&right.2))
                .then_with(|| left.0.cmp(right.0))
        })
        .map(|entry| entry.0)
}

fn evaluate_backend_policy(
    config: &ResolvedConfig,
    provider_id: &str,
) -> Option<BackendPolicyEvaluation> {
    let candidate = BACKEND_POLICY_CANDIDATES
        .iter()
        .find(|candidate| candidate.provider_id.eq_ignore_ascii_case(provider_id))?;

    let policy = &config.backend_selection;
    let mut score = 0_i32;
    if candidate.provider_agnostic {
        score += policy.provider_agnostic_weight;
    }
    if candidate.automation_skills {
        score += policy.automation_skills_weight;
    }
    if candidate.open_source {
        score += policy.open_source_weight;
    }
    if candidate.lsp_support {
        score += policy.lsp_support_weight;
    }
    if candidate.privacy {
        score += policy.privacy_weight;
    }
    if candidate.subscription_required {
        score -= policy.subscription_penalty;
    }

    let preset = provider_preset(candidate.provider_id);
    let env_candidates = collect_provider_api_key_envs(candidate.provider_id, &preset);
    let (api_key, _) = resolve_api_key(config, &env_candidates, candidate.provider_id);
    let requires_api_key = preset.requires_api_key || !env_candidates.is_empty();
    let allowed = config.provider_allowed(candidate.provider_id);
    let available = allowed && (!requires_api_key || api_key.is_some());
    if !available {
        score -= 1000;
    }

    Some(BackendPolicyEvaluation { score, available })
}

pub(crate) fn missing_api_key_message(provider_id: &str, env_candidates: &[String]) -> String {
    if env_candidates.is_empty() {
        format!(
            "provider '{provider_id}' requires an API key; set [llm].api_key_env and export the variable"
        )
    } else {
        format!(
            "provider '{provider_id}' requires an API key; expected one of env vars: {} (or set [llm].api_key_env)",
            env_candidates.join(", ")
        )
    }
}

pub(crate) fn missing_base_url_message(provider_id: &str) -> String {
    format!("provider '{provider_id}' requires an LLM base URL; set [llm].base_url")
}

pub(crate) fn protocol_name(protocol: ProviderProtocol) -> ProviderProtocolName {
    match protocol {
        ProviderProtocol::Null => ProviderProtocolName::Null,
        ProviderProtocol::OpenAiCompatible => ProviderProtocolName::OpenAiCompatible,
        ProviderProtocol::AnthropicMessages => ProviderProtocolName::AnthropicMessages,
        ProviderProtocol::VercelAiGateway => ProviderProtocolName::VercelAiGateway,
        ProviderProtocol::GoogleGenerativeAi => ProviderProtocolName::GoogleGenerativeAi,
    }
}

fn read_config_or_env(config: &ResolvedConfig, name: &str) -> Option<(String, ApiKeySource)> {
    if let Some(value) = config.env.get(name) {
        if !value.trim().is_empty() {
            return Some((
                value.clone(),
                ApiKeySource::ConfigEnvMap {
                    var_name: name.to_string(),
                },
            ));
        }
    }
    let env_value = std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    Some((
        env_value,
        ApiKeySource::ProcessEnv {
            var_name: name.to_string(),
        },
    ))
}

pub(crate) fn parse_model_prefix(model: &str) -> Option<(&str, &str)> {
    let (provider, model_id) = model.split_once('/')?;
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((provider, model_id))
}

pub(crate) fn model_for_provider(provider_id: &str, requested_model: &str) -> String {
    if let Some((candidate_provider, candidate_model)) = parse_model_prefix(requested_model) {
        if candidate_provider.eq_ignore_ascii_case(provider_id)
            || provider_id == "openai-compatible"
        {
            return candidate_model.to_string();
        }
    }
    requested_model.to_string()
}
