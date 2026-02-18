use crate::provider::ProviderProtocol;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub(crate) struct ProviderPreset {
    pub(crate) protocol: ProviderProtocol,
    pub(crate) default_base_url: Option<String>,
    pub(crate) default_api_key_envs: Vec<String>,
    pub(crate) requires_api_key: bool,
}

pub(crate) fn provider_preset(provider_id: &str) -> ProviderPreset {
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
            default_base_url: Some("https://api.githubcopilot.com".to_string()),
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
        "google" => ProviderPreset {
            protocol: ProviderProtocol::GoogleGenerativeAi,
            default_base_url: Some("https://generativelanguage.googleapis.com".to_string()),
            default_api_key_envs: vec![
                "GOOGLE_GENERATIVE_AI_API_KEY".to_string(),
                "GEMINI_API_KEY".to_string(),
            ],
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
        if matches!(preset.protocol, ProviderProtocol::OpenAiCompatible)
            && models_provider
                .npm
                .as_deref()
                .is_some_and(|npm| npm == "@ai-sdk/google")
        {
            preset.protocol = ProviderProtocol::GoogleGenerativeAi;
            if preset.default_base_url.is_none() {
                preset.default_base_url =
                    Some("https://generativelanguage.googleapis.com".to_string());
            }
        }
    }

    preset
}

pub(crate) fn collect_provider_api_key_envs(
    provider_id: &str,
    preset: &ProviderPreset,
) -> Vec<String> {
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
