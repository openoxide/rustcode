use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use rustcode_auth::{AuthStore, StoredCredential};
use rustcode_core::config::ResolvedConfig;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("provider returned invalid response: {0}")]
    Invalid(String),
    #[error("config error: {0}")]
    Config(String),
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;
}

#[derive(Debug, Default)]
pub struct NullLlmClient;

#[async_trait]
impl LlmClient for NullLlmClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            text: format!(
                "null-llm response (model={}): {}",
                request.model, request.prompt
            ),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderProtocol {
    Null,
    OpenAiCompatible,
    AnthropicMessages,
}

#[derive(Debug, Clone)]
struct ProviderPreset {
    protocol: ProviderProtocol,
    default_base_url: Option<String>,
    default_api_key_envs: Vec<String>,
    requires_api_key: bool,
}

#[derive(Debug, Clone)]
struct ResolvedProvider {
    provider_id: String,
    protocol: ProviderProtocol,
    base_url: Option<String>,
    api_key: Option<String>,
    requires_api_key: bool,
    api_key_env_candidates: Vec<String>,
    api_key_source: ApiKeySource,
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
}

pub fn build_client(config: &ResolvedConfig) -> Result<Arc<dyn LlmClient>, LlmError> {
    let provider = resolve_provider(config)?;
    match provider.protocol {
        ProviderProtocol::Null => Ok(Arc::new(NullLlmClient)),
        ProviderProtocol::OpenAiCompatible => {
            if !config.allow_network {
                return Err(LlmError::Config(
                    "network access is disabled; set allow_network=true to use remote LLM providers"
                        .to_string(),
                ));
            }

            let base_url = provider
                .base_url
                .ok_or_else(|| LlmError::Config(missing_base_url_message(&provider.provider_id)))?;
            if provider.requires_api_key && provider.api_key.is_none() {
                return Err(LlmError::Config(missing_api_key_message(
                    &provider.provider_id,
                    &provider.api_key_env_candidates,
                )));
            }

            Ok(Arc::new(OpenAiCompatibleClient::new(
                provider.provider_id,
                base_url,
                provider.api_key,
            )))
        }
        ProviderProtocol::AnthropicMessages => {
            if !config.allow_network {
                return Err(LlmError::Config(
                    "network access is disabled; set allow_network=true to use remote LLM providers"
                        .to_string(),
                ));
            }

            let base_url = provider
                .base_url
                .unwrap_or_else(|| "https://api.anthropic.com".to_string());
            let api_key = provider.api_key.ok_or_else(|| {
                LlmError::Config(missing_api_key_message(
                    &provider.provider_id,
                    &provider.api_key_env_candidates,
                ))
            })?;
            Ok(Arc::new(AnthropicClient::new(
                provider.provider_id,
                base_url,
                api_key,
            )))
        }
    }
}

pub fn diagnose_provider(
    config: &ResolvedConfig,
    provider_id: Option<&str>,
) -> Result<ProviderDiagnostics, LlmError> {
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
    let endpoint = resolved
        .base_url
        .as_ref()
        .map(|base| normalize_endpoint(resolved.protocol, base));
    let mut missing = Vec::new();
    if resolved.base_url.is_none() && !matches!(resolved.protocol, ProviderProtocol::Null) {
        missing.push("base_url".to_string());
    }
    if resolved.requires_api_key && resolved.api_key.is_none() {
        missing.push("api_key".to_string());
    }

    Ok(ProviderDiagnostics {
        provider_id: resolved.provider_id,
        protocol: protocol_name(resolved.protocol),
        base_url: resolved.base_url,
        endpoint,
        requires_api_key: resolved.requires_api_key,
        api_key_source: resolved.api_key_source,
        api_key_env_candidates: resolved.api_key_env_candidates,
        missing,
    })
}

fn resolve_provider(config: &ResolvedConfig) -> Result<ResolvedProvider, LlmError> {
    let (model_provider, _model_id) = parse_model_prefix(&config.model).unwrap_or(("", ""));
    let provider_id = if !config.llm_provider.trim().is_empty() && config.llm_provider != "null" {
        config.llm_provider.clone()
    } else if !model_provider.is_empty() {
        model_provider.to_string()
    } else {
        "null".to_string()
    };

    let preset = provider_preset(&provider_id);
    let base_url = config
        .llm_base_url
        .clone()
        .or_else(|| preset.default_base_url.clone());
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
            StoredCredential::ApiKey { key } => {
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

fn collect_provider_api_key_envs(provider_id: &str, preset: &ProviderPreset) -> Vec<String> {
    let mut names = preset.default_api_key_envs.clone();
    if let Some(meta) = models_provider_metadata(provider_id) {
        for candidate in &meta.env {
            if !names.iter().any(|existing| existing == candidate) {
                names.push(candidate.clone());
            }
        }
    }
    names
}

fn missing_api_key_message(provider_id: &str, env_candidates: &[String]) -> String {
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

fn missing_base_url_message(provider_id: &str) -> String {
    format!("provider '{provider_id}' requires an LLM base URL; set [llm].base_url")
}

fn protocol_name(protocol: ProviderProtocol) -> ProviderProtocolName {
    match protocol {
        ProviderProtocol::Null => ProviderProtocolName::Null,
        ProviderProtocol::OpenAiCompatible => ProviderProtocolName::OpenAiCompatible,
        ProviderProtocol::AnthropicMessages => ProviderProtocolName::AnthropicMessages,
    }
}

fn normalize_endpoint(protocol: ProviderProtocol, base_url: &str) -> String {
    match protocol {
        ProviderProtocol::Null => base_url.to_string(),
        ProviderProtocol::OpenAiCompatible => normalize_openai_chat_endpoint(base_url),
        ProviderProtocol::AnthropicMessages => normalize_anthropic_messages_endpoint(base_url),
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

fn provider_preset(provider_id: &str) -> ProviderPreset {
    let provider = provider_id.to_ascii_lowercase();
    let mut preset = match provider.as_str() {
        "null" => ProviderPreset {
            protocol: ProviderProtocol::Null,
            default_base_url: None,
            default_api_key_envs: vec![],
            requires_api_key: false,
        },
        "anthropic" => ProviderPreset {
            protocol: ProviderProtocol::AnthropicMessages,
            default_base_url: Some("https://api.anthropic.com".to_string()),
            default_api_key_envs: vec!["ANTHROPIC_API_KEY".to_string()],
            requires_api_key: true,
        },
        "ollama" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            default_api_key_envs: vec![],
            requires_api_key: false,
        },
        "openai" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.openai.com/v1".to_string()),
            default_api_key_envs: vec!["OPENAI_API_KEY".to_string()],
            requires_api_key: true,
        },
        "openrouter" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://openrouter.ai/api/v1".to_string()),
            default_api_key_envs: vec!["OPENROUTER_API_KEY".to_string()],
            requires_api_key: true,
        },
        "groq" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.groq.com/openai/v1".to_string()),
            default_api_key_envs: vec!["GROQ_API_KEY".to_string()],
            requires_api_key: true,
        },
        "xai" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.x.ai/v1".to_string()),
            default_api_key_envs: vec!["XAI_API_KEY".to_string()],
            requires_api_key: true,
        },
        "mistral" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.mistral.ai/v1".to_string()),
            default_api_key_envs: vec!["MISTRAL_API_KEY".to_string()],
            requires_api_key: true,
        },
        "togetherai" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.together.xyz/v1".to_string()),
            default_api_key_envs: vec![
                "TOGETHER_API_KEY".to_string(),
                "TOGETHERAI_API_KEY".to_string(),
            ],
            requires_api_key: true,
        },
        "perplexity" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.perplexity.ai".to_string()),
            default_api_key_envs: vec!["PERPLEXITY_API_KEY".to_string()],
            requires_api_key: true,
        },
        "deepinfra" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.deepinfra.com/v1/openai".to_string()),
            default_api_key_envs: vec![
                "DEEPINFRA_API_KEY".to_string(),
                "DEEPINFRA_API_TOKEN".to_string(),
            ],
            requires_api_key: true,
        },
        "cerebras" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.cerebras.ai/v1".to_string()),
            default_api_key_envs: vec!["CEREBRAS_API_KEY".to_string()],
            requires_api_key: true,
        },
        "azure" | "azure-cognitive-services" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: None,
            default_api_key_envs: vec!["AZURE_OPENAI_API_KEY".to_string()],
            requires_api_key: true,
        },
        "github-copilot" | "github-copilot-enterprise" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: None,
            default_api_key_envs: vec![
                "GITHUB_TOKEN".to_string(),
                "GITHUB_COPILOT_TOKEN".to_string(),
            ],
            requires_api_key: true,
        },
        "cloudflare-workers-ai" | "cloudflare-ai-gateway" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: None,
            default_api_key_envs: vec![
                "CLOUDFLARE_API_TOKEN".to_string(),
                "CF_AIG_TOKEN".to_string(),
            ],
            requires_api_key: true,
        },
        "google-vertex"
        | "google-vertex-anthropic"
        | "amazon-bedrock"
        | "gitlab"
        | "opencode"
        | "sap-ai-core"
        | "vercel"
        | "zenmux"
        | "fetch" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: None,
            default_api_key_envs: vec![],
            requires_api_key: false,
        },
        _ => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: None,
            default_api_key_envs: vec![],
            requires_api_key: false,
        },
    };

    if let Some(models_provider) = models_provider_metadata(&provider) {
        if preset.default_base_url.is_none() {
            preset.default_base_url = models_provider.api.clone();
        }
        if preset.default_api_key_envs.is_empty() {
            preset.default_api_key_envs = models_provider.env.clone();
        }
        if !preset.requires_api_key && !preset.default_api_key_envs.is_empty() {
            preset.requires_api_key = true;
        }
        if matches!(preset.protocol, ProviderProtocol::OpenAiCompatible)
            && models_provider
                .npm
                .as_deref()
                .is_some_and(|npm| npm.contains("anthropic"))
        {
            preset.protocol = ProviderProtocol::AnthropicMessages;
        }
    }

    preset
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsProviderMetadata {
    api: Option<String>,
    npm: Option<String>,
    #[serde(default)]
    env: Vec<String>,
}

static MODELS_PROVIDER_INDEX: OnceLock<Option<BTreeMap<String, ModelsProviderMetadata>>> =
    OnceLock::new();

fn models_provider_metadata(provider_id: &str) -> Option<&'static ModelsProviderMetadata> {
    let index = MODELS_PROVIDER_INDEX
        .get_or_init(load_models_provider_index)
        .as_ref()?;
    index.get(provider_id)
}

fn load_models_provider_index() -> Option<BTreeMap<String, ModelsProviderMetadata>> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("RUSTCODE_MODELS_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join(".cache/opencode/models.json"));
        candidates.push(PathBuf::from(home).join(".opencode/models.json"));
    }
    if let Ok(cache_home) = std::env::var("XDG_CACHE_HOME") {
        candidates.push(PathBuf::from(cache_home).join("opencode/models.json"));
    }

    for candidate in candidates {
        let Ok(raw) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<BTreeMap<String, ModelsProviderMetadata>>(&raw)
        else {
            continue;
        };
        return Some(parsed);
    }
    None
}

fn parse_model_prefix(model: &str) -> Option<(&str, &str)> {
    let (provider, model_id) = model.split_once('/')?;
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((provider, model_id))
}

#[derive(Debug)]
struct OpenAiCompatibleClient {
    provider_id: String,
    endpoint: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatibleClient {
    fn new(provider_id: String, base_url: String, api_key: Option<String>) -> Self {
        Self {
            provider_id,
            endpoint: normalize_openai_chat_endpoint(&base_url),
            api_key,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "messages": [{"role":"user", "content": request.prompt}],
            }))
            .send()
            .await
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| LlmError::Transport(err.to_string()))?;
        if !status.is_success() {
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        let parsed: Value = serde_json::from_str(&body)
            .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
        let text = extract_openai_text(&parsed).ok_or_else(|| {
            LlmError::Invalid(
                "choices[0].message.content missing from provider response".to_string(),
            )
        })?;
        Ok(LlmResponse { text })
    }
}

#[derive(Debug)]
struct AnthropicClient {
    provider_id: String,
    endpoint: String,
    api_key: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    fn new(provider_id: String, base_url: String, api_key: String) -> Self {
        Self {
            provider_id,
            endpoint: normalize_anthropic_messages_endpoint(&base_url),
            api_key,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(&self.api_key)
                .map_err(|err| LlmError::Config(format!("invalid anthropic key header: {err}")))?,
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": request.prompt}],
            }))
            .send()
            .await
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| LlmError::Transport(err.to_string()))?;
        if !status.is_success() {
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        let parsed: Value = serde_json::from_str(&body)
            .map_err(|err| LlmError::Invalid(format!("response is not valid JSON: {err}")))?;
        let text = extract_anthropic_text(&parsed).ok_or_else(|| {
            LlmError::Invalid("content[0].text missing from anthropic response".to_string())
        })?;
        Ok(LlmResponse { text })
    }
}

fn model_for_provider(provider_id: &str, requested_model: &str) -> String {
    if let Some((candidate_provider, candidate_model)) = parse_model_prefix(requested_model) {
        if candidate_provider.eq_ignore_ascii_case(provider_id)
            || provider_id == "openai-compatible"
        {
            return candidate_model.to_string();
        }
    }
    requested_model.to_string()
}

fn normalize_openai_chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn normalize_anthropic_messages_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/messages")
    } else {
        format!("{trimmed}/v1/messages")
    }
}

fn extract_openai_text(value: &Value) -> Option<String> {
    let message = value.get("choices")?.get(0)?;
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let content = message.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    let content_parts = content.as_array()?;
    let mut combined = String::new();
    for part in content_parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            combined.push_str(text);
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn extract_anthropic_text(value: &Value) -> Option<String> {
    let content = value.get("content")?.as_array()?;
    let mut combined = String::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
    }
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 320;
    if body.len() <= LIMIT {
        body.to_string()
    } else {
        format!("{}...", &body[..LIMIT])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_auth::AuthStore;
    use rustcode_core::config::ResolvedConfig;
    use std::path::PathBuf;
    use std::sync::{LazyLock, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn resolves_provider_from_model_prefix_when_provider_is_null() {
        let cfg = ResolvedConfig {
            allow_network: true,
            model: "openrouter/openai/gpt-5-mini".to_string(),
            ..ResolvedConfig::default()
        };

        let provider = resolve_provider(&cfg).expect("provider must resolve");
        assert_eq!(provider.provider_id, "openrouter");
        assert_eq!(provider.protocol, ProviderProtocol::OpenAiCompatible);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(provider.api_key_source, ApiKeySource::None);
    }

    #[test]
    fn model_prefix_is_removed_for_matching_provider() {
        let model = model_for_provider("openrouter", "openrouter/openai/gpt-5-mini");
        assert_eq!(model, "openai/gpt-5-mini");
    }

    #[test]
    fn openai_endpoint_normalization_handles_v1_and_full_path() {
        assert_eq!(
            normalize_openai_chat_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_openai_chat_endpoint("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            normalize_openai_chat_endpoint("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_endpoint_normalization_handles_v1_and_full_path() {
        assert_eq!(
            normalize_anthropic_messages_endpoint("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            normalize_anthropic_messages_endpoint("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn parses_openai_string_and_array_content() {
        let string_content = json!({
            "choices": [{"message": {"content": "hello"}}]
        });
        assert_eq!(
            extract_openai_text(&string_content).as_deref(),
            Some("hello")
        );

        let array_content = json!({
            "choices": [{"message": {"content": [{"type":"text","text":"he"},{"type":"text","text":"llo"}]}}]
        });
        assert_eq!(
            extract_openai_text(&array_content).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parses_anthropic_text_content() {
        let payload = json!({
            "content": [
                {"type":"text","text":"hello"},
                {"type":"thinking","text":"ignored"},
                {"type":"text","text":" world"}
            ]
        });

        assert_eq!(
            extract_anthropic_text(&payload).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn resolves_api_key_from_auth_store_when_env_missing() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        let auth_path = make_temp_file_path("llm-auth-store");
        let store = AuthStore::with_path(auth_path.clone());
        store
            .set_api_key("openrouter", "stored-secret")
            .expect("must write auth key");

        std::env::set_var("RUSTCODE_AUTH_FILE", &auth_path);
        let cfg = ResolvedConfig {
            allow_network: true,
            llm_provider: "openrouter".to_string(),
            ..ResolvedConfig::default()
        };

        let provider = resolve_provider(&cfg).expect("provider must resolve");
        assert_eq!(provider.api_key.as_deref(), Some("stored-secret"));
        assert_eq!(provider.api_key_source, ApiKeySource::AuthStore);
        std::env::remove_var("RUSTCODE_AUTH_FILE");
    }

    #[test]
    fn resolves_oauth_access_from_auth_store_when_env_missing() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        let auth_path = make_temp_file_path("llm-auth-store-oauth");
        let store = AuthStore::with_path(auth_path.clone());
        store
            .set_oauth(
                "openai",
                "oauth-access-token",
                Some("oauth-refresh-token"),
                Some(1234567890),
                Some("acct_123"),
            )
            .expect("must write oauth credential");

        std::env::set_var("RUSTCODE_AUTH_FILE", &auth_path);
        std::env::remove_var("OPENAI_API_KEY");

        let cfg = ResolvedConfig {
            allow_network: true,
            llm_provider: "openai".to_string(),
            ..ResolvedConfig::default()
        };

        let provider = resolve_provider(&cfg).expect("provider must resolve");
        assert_eq!(provider.api_key.as_deref(), Some("oauth-access-token"));
        assert_eq!(provider.api_key_source, ApiKeySource::AuthStore);
        std::env::remove_var("RUSTCODE_AUTH_FILE");
    }

    #[test]
    fn diagnostics_include_missing_key_when_not_configured() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        std::env::remove_var("OPENROUTER_API_KEY");
        let cfg = ResolvedConfig {
            allow_network: true,
            llm_provider: "openrouter".to_string(),
            ..ResolvedConfig::default()
        };

        let diag = diagnose_provider(&cfg, Some("openrouter")).expect("diagnostic should resolve");
        assert_eq!(diag.provider_id, "openrouter");
        assert_eq!(diag.protocol, ProviderProtocolName::OpenAiCompatible);
        assert!(diag.endpoint.is_some());
        assert!(diag.requires_api_key);
        assert!(diag.missing.iter().any(|item| item == "api_key"));
    }

    fn make_temp_file_path(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        let pid = std::process::id();
        std::env::temp_dir().join(format!("rustcode-llm-{name}-{pid}-{now}.json"))
    }
}
