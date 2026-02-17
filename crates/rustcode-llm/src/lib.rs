use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use rustcode_auth::{AuthStore, StoredCredential};
use rustcode_core::config::ResolvedConfig;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Value,
    // Tool result messages use `tool_call_id`. Assistant messages that trigger tools should
    // populate `tool_calls` so the next turn can reference them.
    pub tool_call_id: Option<String>,
    // Some provider protocols require the tool name when returning a tool result (e.g., Gemini).
    pub tool_name: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub chunks: Vec<String>,
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

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::Invalid(
            "chat is not supported by the active provider client".to_string(),
        ))
    }
}

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_RESPONSE_BODY_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn llm_http_client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .build()
        .map_err(|err| LlmError::Transport(format!("failed to build http client: {err}")))
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
            chunks: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderProtocol {
    Null,
    OpenAiCompatible,
    AnthropicMessages,
    VercelAiGateway,
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
    VercelAiGateway,
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
            )?))
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
            )?))
        }
        ProviderProtocol::VercelAiGateway => {
            if !config.allow_network {
                return Err(LlmError::Config(
                    "network access is disabled; set allow_network=true to use remote LLM providers"
                        .to_string(),
                ));
            }

            let base_url = provider
                .base_url
                .unwrap_or_else(|| "https://ai-gateway.vercel.sh/v1/ai".to_string());
            let api_key = provider.api_key.ok_or_else(|| {
                LlmError::Config(missing_api_key_message(
                    &provider.provider_id,
                    &provider.api_key_env_candidates,
                ))
            })?;
            Ok(Arc::new(VercelAiGatewayClient::new(
                provider.provider_id,
                base_url,
                api_key,
            )?))
        }
    }
}

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

fn resolve_provider(config: &ResolvedConfig) -> Result<ResolvedProvider, LlmError> {
    let (model_provider, _model_id) = parse_model_prefix(&config.model).unwrap_or(("", ""));
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
        ProviderProtocol::VercelAiGateway => ProviderProtocolName::VercelAiGateway,
    }
}

fn normalize_endpoint(protocol: ProviderProtocol, base_url: &str) -> String {
    match protocol {
        ProviderProtocol::Null => base_url.to_string(),
        ProviderProtocol::OpenAiCompatible => normalize_openai_chat_endpoint(base_url),
        ProviderProtocol::AnthropicMessages => normalize_anthropic_messages_endpoint(base_url),
        ProviderProtocol::VercelAiGateway => normalize_vercel_gateway_endpoint(base_url),
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
        "v0" => ProviderPreset {
            protocol: ProviderProtocol::OpenAiCompatible,
            default_base_url: Some("https://api.v0.dev/v1".to_string()),
            default_api_key_envs: vec!["V0_API_KEY".to_string()],
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
        "vercel" => ProviderPreset {
            protocol: ProviderProtocol::VercelAiGateway,
            default_base_url: Some("https://ai-gateway.vercel.sh/v1/ai".to_string()),
            default_api_key_envs: vec!["AI_GATEWAY_API_KEY".to_string()],
            requires_api_key: true,
        },
        "google-vertex"
        | "google-vertex-anthropic"
        | "amazon-bedrock"
        | "gitlab"
        | "opencode"
        | "sap-ai-core"
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
    fn new(
        provider_id: String,
        base_url: String,
        api_key: Option<String>,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            endpoint: normalize_openai_chat_endpoint(&base_url),
            api_key,
            http: llm_http_client()?,
        })
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
        apply_provider_default_headers(&self.provider_id, &mut headers)
            .map_err(|err| LlmError::Config(err))?;

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "messages": [{"role":"user", "content": request.prompt}],
                "stream": true,
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
                .await
                .map_err(|_| {
                    LlmError::Transport("timed out reading provider error body".to_string())
                })?
                .map_err(|err| LlmError::Transport(err.to_string()))?;
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
            StreamedProviderBody::Body(body) => {
                let parsed: Value = serde_json::from_str(&body).map_err(|err| {
                    LlmError::Invalid(format!("response is not valid JSON: {err}"))
                })?;
                let text = extract_openai_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid(
                        "choices[0].message.content missing from provider response".to_string(),
                    )
                })?;
                Ok(LlmResponse {
                    text,
                    chunks: Vec::new(),
                })
            }
            StreamedProviderBody::SseEvents(events) => {
                let mut chunks = Vec::new();
                let mut combined = String::new();
                for event in events {
                    if event == "[DONE]" {
                        break;
                    }
                    let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(error_message) = extract_stream_error_message(&parsed) {
                        return Err(LlmError::Transport(format!(
                            "provider stream error: {error_message}"
                        )));
                    }
                    if let Some(delta) = extract_openai_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "openai-compatible stream did not include text deltas".to_string(),
                    ));
                }
                Ok(LlmResponse {
                    text: combined,
                    chunks,
                })
            }
        }
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &self.api_key {
            let bearer = format!("Bearer {api_key}");
            let value = HeaderValue::from_str(&bearer)
                .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
            headers.insert(AUTHORIZATION, value);
        }
        apply_provider_default_headers(&self.provider_id, &mut headers)
            .map_err(LlmError::Config)?;

        let messages = request
            .messages
            .iter()
            .map(openai_message_value)
            .collect::<Vec<_>>();
        let tools = request
            .tools
            .iter()
            .map(openai_tool_spec_value)
            .collect::<Vec<_>>();

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "messages": messages,
                "tools": tools,
                "tool_choice": "auto",
                "stream": false,
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
            .await
            .map_err(|_| LlmError::Transport("timed out reading provider body".to_string()))?
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
        let tool_calls = extract_openai_tool_calls(&parsed);
        let text = extract_openai_text(&parsed).unwrap_or_default();
        if text.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::Invalid(
                "provider response did not include content or tool calls".to_string(),
            ));
        }

        Ok(ChatResponse { text, tool_calls })
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
    fn new(provider_id: String, base_url: String, api_key: String) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            endpoint: normalize_anthropic_messages_endpoint(&base_url),
            api_key,
            http: llm_http_client()?,
        })
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
        // Align with OpenCode's default Anthropic headers for Claude Code compatibility.
        headers.insert(
            HeaderName::from_static("anthropic-beta"),
            HeaderValue::from_static(
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
            ),
        );

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": request.prompt}],
                "stream": true,
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
                .await
                .map_err(|_| {
                    LlmError::Transport("timed out reading provider error body".to_string())
                })?
                .map_err(|err| LlmError::Transport(err.to_string()))?;
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
            StreamedProviderBody::Body(body) => {
                let parsed: Value = serde_json::from_str(&body).map_err(|err| {
                    LlmError::Invalid(format!("response is not valid JSON: {err}"))
                })?;
                let text = extract_anthropic_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid("content[0].text missing from anthropic response".to_string())
                })?;
                Ok(LlmResponse {
                    text,
                    chunks: Vec::new(),
                })
            }
            StreamedProviderBody::SseEvents(events) => {
                let mut chunks = Vec::new();
                let mut combined = String::new();
                for event in events {
                    let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(error_message) = extract_stream_error_message(&parsed) {
                        return Err(LlmError::Transport(format!(
                            "provider stream error: {error_message}"
                        )));
                    }
                    if let Some(delta) = extract_anthropic_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "anthropic stream did not include text deltas".to_string(),
                    ));
                }
                Ok(LlmResponse {
                    text: combined,
                    chunks,
                })
            }
        }
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
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
        // Align with OpenCode's default Anthropic headers for Claude Code compatibility.
        headers.insert(
            HeaderName::from_static("anthropic-beta"),
            HeaderValue::from_static(
                "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
            ),
        );

        let (system, messages) = anthropic_messages_from_chat(&request)?;
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "model": model,
                "max_tokens": 1024,
                "system": system,
                "messages": messages,
                "tools": tools,
                "stream": false,
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
            .await
            .map_err(|_| LlmError::Transport("timed out reading provider body".to_string()))?
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
        let tool_calls = extract_anthropic_tool_calls(&parsed);
        let text = extract_anthropic_text(&parsed).unwrap_or_default();
        if text.is_empty() && tool_calls.is_empty() {
            return Err(LlmError::Invalid(
                "provider response did not include content or tool calls".to_string(),
            ));
        }

        Ok(ChatResponse { text, tool_calls })
    }
}

#[derive(Debug)]
struct VercelAiGatewayClient {
    provider_id: String,
    endpoint: String,
    api_key: String,
    http: reqwest::Client,
}

impl VercelAiGatewayClient {
    fn new(provider_id: String, base_url: String, api_key: String) -> Result<Self, LlmError> {
        Ok(Self {
            provider_id,
            endpoint: normalize_vercel_gateway_endpoint(&base_url),
            api_key,
            http: llm_http_client()?,
        })
    }
}

#[async_trait]
impl LlmClient for VercelAiGatewayClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let model = model_for_provider(&self.provider_id, &request.model);

        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {}", self.api_key);
        let value = HeaderValue::from_str(&bearer)
            .map_err(|err| LlmError::Config(format!("invalid auth header: {err}")))?;
        headers.insert(AUTHORIZATION, value);
        headers.insert(
            HeaderName::from_static("ai-gateway-protocol-version"),
            HeaderValue::from_static("0.0.1"),
        );
        headers.insert(
            HeaderName::from_static("ai-gateway-auth-method"),
            HeaderValue::from_static("api-key"),
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-specification-version"),
            HeaderValue::from_static("2"),
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-id"),
            HeaderValue::from_str(&model)
                .map_err(|err| LlmError::Config(format!("invalid model header: {err}")))?,
        );
        headers.insert(
            HeaderName::from_static("ai-language-model-streaming"),
            HeaderValue::from_static("true"),
        );
        apply_provider_default_headers("vercel", &mut headers)
            .map_err(|err| LlmError::Config(err))?;

        let response = self
            .http
            .post(&self.endpoint)
            .headers(headers)
            .json(&json!({
                "prompt": [{
                    "role": "user",
                    "content": [{"type": "text", "text": request.prompt}],
                }],
            }))
            .send();
        let response = timeout(HTTP_RESPONSE_HEADER_TIMEOUT, response)
            .await
            .map_err(|_| {
                LlmError::Transport("timed out waiting for provider response headers".to_string())
            })?
            .map_err(|err| LlmError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = timeout(HTTP_RESPONSE_BODY_TIMEOUT, response.text())
                .await
                .map_err(|_| {
                    LlmError::Transport("timed out reading provider error body".to_string())
                })?
                .map_err(|err| LlmError::Transport(err.to_string()))?;
            return Err(LlmError::Transport(format!(
                "provider returned {}: {}",
                status,
                truncate_for_error(&body)
            )));
        }

        match read_sse_or_body(response, HTTP_STREAM_IDLE_TIMEOUT).await? {
            StreamedProviderBody::Body(body) => {
                let parsed: Value = serde_json::from_str(&body).map_err(|err| {
                    LlmError::Invalid(format!("response is not valid JSON: {err}"))
                })?;
                let text = extract_gateway_text(&parsed).ok_or_else(|| {
                    LlmError::Invalid("gateway response did not include text".to_string())
                })?;
                Ok(LlmResponse {
                    text,
                    chunks: Vec::new(),
                })
            }
            StreamedProviderBody::SseEvents(events) => {
                let mut chunks = Vec::new();
                let mut combined = String::new();
                for event in events {
                    let Ok(parsed) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(error_message) = extract_stream_error_message(&parsed) {
                        return Err(LlmError::Transport(format!(
                            "provider stream error: {error_message}"
                        )));
                    }
                    if let Some(delta) = extract_gateway_stream_delta(&parsed) {
                        if !delta.is_empty() {
                            combined.push_str(&delta);
                            chunks.push(delta);
                        }
                    }
                }
                if combined.is_empty() {
                    return Err(LlmError::Invalid(
                        "gateway stream did not include text deltas".to_string(),
                    ));
                }
                Ok(LlmResponse {
                    text: combined,
                    chunks,
                })
            }
        }
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

fn openai_role_value(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

fn openai_tool_call_value(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": call.arguments,
        }
    })
}

fn openai_message_value(message: &ChatMessage) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "role".to_string(),
        Value::String(openai_role_value(message.role).to_string()),
    );
    obj.insert("content".to_string(), message.content.clone());
    if message.role == ChatRole::Tool {
        if let Some(tool_call_id) = message.tool_call_id.as_ref() {
            obj.insert(
                "tool_call_id".to_string(),
                Value::String(tool_call_id.clone()),
            );
        }
        // OpenAI-style tool results do not require tool name, but keep it if present
        // for interoperability with proxies that validate the field.
        if let Some(tool_name) = message.tool_name.as_ref() {
            obj.insert("name".to_string(), Value::String(tool_name.clone()));
        }
    }
    if message.role == ChatRole::Assistant && !message.tool_calls.is_empty() {
        obj.insert(
            "tool_calls".to_string(),
            Value::Array(
                message
                    .tool_calls
                    .iter()
                    .map(openai_tool_call_value)
                    .collect(),
            ),
        );
    }
    Value::Object(obj)
}

fn openai_tool_spec_value(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        }
    })
}

fn apply_provider_default_headers(
    provider_id: &str,
    headers: &mut HeaderMap,
) -> Result<(), String> {
    // These headers are not required by the OpenAI-compatible spec, but several providers
    // use them for attribution / routing. Align with OpenCode defaults.
    match provider_id {
        "openrouter" => {
            headers.insert(
                HeaderName::from_static("http-referer"),
                HeaderValue::from_static("https://opencode.ai/"),
            );
            headers.insert(
                HeaderName::from_static("x-title"),
                HeaderValue::from_static("opencode"),
            );
        }
        "vercel" => {
            headers.insert(
                HeaderName::from_static("http-referer"),
                HeaderValue::from_static("https://opencode.ai/"),
            );
            headers.insert(
                HeaderName::from_static("x-title"),
                HeaderValue::from_static("opencode"),
            );
        }
        _ => {}
    }
    Ok(())
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

fn normalize_vercel_gateway_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/language-model") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/language-model")
    }
}

enum StreamedProviderBody {
    Body(String),
    SseEvents(Vec<String>),
}

async fn read_sse_or_body(
    response: reqwest::Response,
    idle_timeout: Duration,
) -> Result<StreamedProviderBody, LlmError> {
    let mut stream = response.bytes_stream();
    let mut raw_body = String::new();
    let mut parse_buffer = String::new();
    let mut events = Vec::new();

    loop {
        let chunk = timeout(idle_timeout, stream.next()).await.map_err(|_| {
            LlmError::Transport("timed out waiting for provider response chunk".to_string())
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|err| LlmError::Transport(err.to_string()))?;
        let text = String::from_utf8_lossy(&chunk);
        raw_body.push_str(&text);
        parse_buffer.push_str(&text);

        while let Some(block) = take_next_sse_block(&mut parse_buffer) {
            if let Some(data) = parse_sse_data_block(&block) {
                events.push(data);
            }
        }
    }

    if events.is_empty() {
        return Ok(StreamedProviderBody::Body(raw_body));
    }

    if let Some(trailing) = parse_sse_data_block(parse_buffer.trim()) {
        events.push(trailing);
    }
    Ok(StreamedProviderBody::SseEvents(events))
}

fn take_next_sse_block(buffer: &mut String) -> Option<String> {
    let mut separator: Option<(usize, usize)> = None;

    if let Some(index) = buffer.find("\n\n") {
        separator = Some((index, 2));
    }
    if let Some(index) = buffer.find("\r\n\r\n") {
        match separator {
            Some((current, _)) if current <= index => {}
            _ => separator = Some((index, 4)),
        }
    }

    let (index, length) = separator?;
    let block = buffer[..index].to_string();
    buffer.drain(..index + length);
    Some(block)
}

fn parse_sse_data_block(block: &str) -> Option<String> {
    if block.is_empty() {
        return None;
    }

    let mut data_lines = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

fn extract_stream_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    if let Some(message) = error.as_str() {
        return Some(message.to_string());
    }
    Some(error.to_string())
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

fn extract_openai_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(message) = value.get("choices").and_then(|choices| choices.get(0)) else {
        return Vec::new();
    };
    let Some(message) = message.get("message") else {
        return Vec::new();
    };
    let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut parsed = Vec::new();
    for call in tool_calls {
        let Some(id) = call.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(function) = call.get("function") else {
            continue;
        };
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(arguments) = function.get("arguments") else {
            continue;
        };
        let arguments = match arguments {
            Value::String(raw) => raw.clone(),
            other => other.to_string(),
        };
        parsed.push(ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        });
    }
    parsed
}

fn extract_openai_stream_delta(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;
    if let Some(text) = choice.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    let delta = choice.get("delta")?;
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    if let Some(parts) = delta.get("content").and_then(Value::as_array) {
        let mut combined = String::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
        if !combined.is_empty() {
            return Some(combined);
        }
    }

    None
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

fn extract_anthropic_tool_calls(value: &Value) -> Vec<ToolCall> {
    let Some(content) = value.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut calls = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let input = item.get("input").cloned().unwrap_or(Value::Null);
        calls.push(ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: input.to_string(),
        });
    }
    calls
}

fn extract_anthropic_stream_delta(value: &Value) -> Option<String> {
    let event_type = value.get("type").and_then(Value::as_str)?;
    match event_type {
        "content_block_delta" => value
            .get("delta")
            .and_then(|delta| delta.get("text"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "content_block_start" => value
            .get("content_block")
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

fn anthropic_messages_from_chat(request: &ChatRequest) -> Result<(String, Vec<Value>), LlmError> {
    let mut system = String::new();
    let mut messages: Vec<Value> = Vec::new();

    for message in &request.messages {
        match message.role {
            ChatRole::System => {
                if let Some(text) = message.content.as_str() {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(text);
                }
            }
            ChatRole::User => {
                let blocks = anthropic_content_blocks_from_value(&message.content);
                if !blocks.is_empty() {
                    messages.push(json!({"role":"user","content": blocks}));
                }
            }
            ChatRole::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message.content.as_str() {
                    if !text.is_empty() {
                        blocks.push(json!({"type":"text","text": text}));
                    }
                }
                for call in &message.tool_calls {
                    let input: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                if !blocks.is_empty() {
                    messages.push(json!({"role":"assistant","content": blocks}));
                }
            }
            ChatRole::Tool => {
                let Some(call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                let (content, is_error) = anthropic_tool_result_from_value(&message.content);
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                        "is_error": is_error
                    }]
                }));
            }
        }
    }

    Ok((system, messages))
}

fn anthropic_content_blocks_from_value(value: &Value) -> Vec<Value> {
    if let Some(text) = value.as_str() {
        if text.is_empty() {
            return Vec::new();
        }
        return vec![json!({"type":"text","text": text})];
    }
    if value.is_null() {
        return Vec::new();
    }
    vec![json!({"type":"text","text": value.to_string()})]
}

fn anthropic_tool_result_from_value(value: &Value) -> (String, bool) {
    let content = if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    };

    let is_error = match serde_json::from_str::<Value>(&content) {
        Ok(parsed) => parsed
            .get("ok")
            .and_then(Value::as_bool)
            .map(|ok| !ok)
            .unwrap_or(false),
        Err(_) => false,
    };

    (content, is_error)
}

fn truncate_for_error(body: &str) -> String {
    const LIMIT: usize = 320;
    if body.len() <= LIMIT {
        body.to_string()
    } else {
        format!("{}...", &body[..LIMIT])
    }
}

fn extract_gateway_text(value: &Value) -> Option<String> {
    if let Some(text) = extract_openai_text(value) {
        return Some(text);
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = value.get("output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    None
}

fn extract_gateway_stream_delta(value: &Value) -> Option<String> {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    let candidates = ["textDelta", "delta", "text"];

    if kind.contains("delta") || kind.contains("text") {
        for key in candidates {
            if let Some(text) = value.get(key).and_then(Value::as_str) {
                return Some(text.to_string());
            }
        }
    }

    None
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
    fn vercel_gateway_endpoint_normalization_appends_language_model_path() {
        assert_eq!(
            normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai"),
            "https://ai-gateway.vercel.sh/v1/ai/language-model"
        );
        assert_eq!(
            normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai/"),
            "https://ai-gateway.vercel.sh/v1/ai/language-model"
        );
        assert_eq!(
            normalize_vercel_gateway_endpoint("https://ai-gateway.vercel.sh/v1/ai/language-model"),
            "https://ai-gateway.vercel.sh/v1/ai/language-model"
        );
    }

    #[test]
    fn gateway_stream_delta_extracts_text_delta() {
        let payload = json!({
            "type": "text-delta",
            "textDelta": "hello"
        });
        assert_eq!(
            extract_gateway_stream_delta(&payload).as_deref(),
            Some("hello")
        );
        let metadata = json!({
            "type": "response-metadata",
            "timestamp": "2026-02-17T00:00:00Z"
        });
        assert!(extract_gateway_stream_delta(&metadata).is_none());
    }

    #[test]
    fn vercel_provider_resolves_as_gateway_protocol_and_requires_key() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        std::env::remove_var("AI_GATEWAY_API_KEY");
        let cfg = ResolvedConfig {
            allow_network: true,
            llm_provider: "vercel".to_string(),
            ..ResolvedConfig::default()
        };
        let diag = diagnose_provider(&cfg, Some("vercel")).expect("diagnostic should resolve");
        assert_eq!(diag.protocol, ProviderProtocolName::VercelAiGateway);
        assert!(diag.requires_api_key);
        assert!(diag.missing.iter().any(|item| item == "api_key"));
        assert_eq!(
            diag.endpoint.as_deref(),
            Some("https://ai-gateway.vercel.sh/v1/ai/language-model")
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
    fn parses_anthropic_tool_use_blocks() {
        let payload = json!({
            "content": [
                {"type":"tool_use","id":"toolu_1","name":"read","input":{"path":"README.md"}},
                {"type":"text","text":"ok"}
            ]
        });

        let calls = extract_anthropic_tool_calls(&payload);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].name, "read");
        assert!(calls[0].arguments.contains("README.md"));
    }

    #[test]
    fn parses_openai_stream_deltas() {
        let string_delta = json!({
            "choices": [{"delta": {"content": "hello"}}]
        });
        assert_eq!(
            extract_openai_stream_delta(&string_delta).as_deref(),
            Some("hello")
        );

        let array_delta = json!({
            "choices": [{"delta": {"content": [{"type":"text","text":"he"},{"type":"text","text":"llo"}]}}]
        });
        assert_eq!(
            extract_openai_stream_delta(&array_delta).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parses_openai_tool_calls_from_response() {
        let payload = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        });

        let calls = extract_openai_tool_calls(&payload);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "read");
        assert!(calls[0].arguments.contains("README.md"));
    }

    #[test]
    fn parses_anthropic_stream_deltas() {
        let start = json!({
            "type": "content_block_start",
            "content_block": {"type": "text", "text": "hello"}
        });
        assert_eq!(
            extract_anthropic_stream_delta(&start).as_deref(),
            Some("hello")
        );

        let delta = json!({
            "type": "content_block_delta",
            "delta": {"type":"text_delta","text":" world"}
        });
        assert_eq!(
            extract_anthropic_stream_delta(&delta).as_deref(),
            Some(" world")
        );
    }

    #[test]
    fn parses_sse_blocks_with_lf_and_crlf() {
        let mut lf = "data: one\n\ndata: two\n\n".to_string();
        let first = take_next_sse_block(&mut lf).expect("first block");
        let second = take_next_sse_block(&mut lf).expect("second block");
        assert_eq!(parse_sse_data_block(&first).as_deref(), Some("one"));
        assert_eq!(parse_sse_data_block(&second).as_deref(), Some("two"));

        let mut crlf = "data: a\r\n\r\ndata: b\r\n\r\n".to_string();
        let first = take_next_sse_block(&mut crlf).expect("first block");
        let second = take_next_sse_block(&mut crlf).expect("second block");
        assert_eq!(parse_sse_data_block(&first).as_deref(), Some("a"));
        assert_eq!(parse_sse_data_block(&second).as_deref(), Some("b"));
    }

    #[test]
    fn extracts_stream_error_message_from_object_payload() {
        let payload = json!({
            "error": {"message": "insufficient_quota"}
        });
        assert_eq!(
            extract_stream_error_message(&payload).as_deref(),
            Some("insufficient_quota")
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

    #[test]
    fn policy_selection_prefers_available_backend() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        std::env::remove_var("OPENROUTER_API_KEY");
        std::env::set_var("OPENAI_API_KEY", "policy-openai-key");

        let cfg = ResolvedConfig {
            allow_network: true,
            model: "gpt-5".to_string(),
            ..ResolvedConfig::default()
        };

        let provider = resolve_provider(&cfg).expect("provider must resolve");
        assert_eq!(provider.provider_id, "openai");
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn policy_diagnostics_surface_score_and_selection() {
        let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
        std::env::set_var("OPENROUTER_API_KEY", "policy-openrouter-key");
        std::env::remove_var("OPENAI_API_KEY");

        let cfg = ResolvedConfig {
            allow_network: true,
            model: "gpt-5".to_string(),
            ..ResolvedConfig::default()
        };

        let openrouter =
            diagnose_provider(&cfg, Some("openrouter")).expect("diagnostic should resolve");
        assert_eq!(openrouter.policy_score, Some(6));
        assert_eq!(openrouter.policy_available, Some(true));
        assert!(openrouter.policy_selected);

        let openai = diagnose_provider(&cfg, Some("openai")).expect("diagnostic should resolve");
        assert_eq!(openai.policy_score, Some(-998));
        assert_eq!(openai.policy_available, Some(false));
        assert!(!openai.policy_selected);

        std::env::remove_var("OPENROUTER_API_KEY");
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
