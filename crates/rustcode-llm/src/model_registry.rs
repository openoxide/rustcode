// ── Model registry ────────────────────────────────────────────────────
//
// Centralizes model metadata: context window sizes, output limits,
// and capability flags. Single source of truth for all model lookups.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

/// Capabilities and limits for a known model.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Canonical model identifier (e.g. "claude-3.5-sonnet").
    pub id: String,
    /// Provider that serves this model.
    pub provider: String,
    /// Maximum context window in tokens.
    pub context_window: u64,
    /// Maximum output tokens per response.
    pub max_output_tokens: u64,
    /// Whether the model supports tool/function calling.
    pub supports_tools: bool,
    /// Whether the model supports image/vision inputs.
    pub supports_vision: bool,
    /// Whether the model supports streaming responses.
    pub supports_streaming: bool,
}

impl Default for ModelInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider: String::new(),
            context_window: 128_000,
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
        }
    }
}

/// Look up metadata for a model by its identifier.
///
/// Checks the external models index first, then falls back to built-in
/// presets. Returns a default for completely unknown models.
#[must_use]
pub fn model_info(model_id: &str) -> ModelInfo {
    let lower = model_id.to_ascii_lowercase();

    // 1. Check external index
    if let Some(ext) = external_model_info(&lower) {
        return ext;
    }

    // 2. Built-in presets
    builtin_model_info(&lower)
}

/// Return the context window limit for a model.
///
/// Convenience wrapper used by `ContextTracker`.
#[must_use]
pub fn context_limit(model_id: &str) -> u64 {
    model_info(model_id).context_window
}

/// List all known built-in model identifiers for a provider.
#[must_use]
pub fn list_models(provider: &str) -> Vec<&'static str> {
    let p = provider.to_ascii_lowercase();
    BUILTIN_MODELS
        .iter()
        .filter(|(_, info)| info.2 == p)
        .map(|(id, _)| *id)
        .collect()
}

// ── Built-in model presets ─────────────────────────────────────────

/// (`context_window`, `max_output`, provider, tools, vision, streaming)
type Preset = (u64, u64, &'static str, bool, bool, bool);

/// Built-in model presets: (context, `max_output`, provider, tools, vision, streaming)
static BUILTIN_MODELS: &[(&str, Preset)] = &[
    // Anthropic Claude
    (
        "claude-3.5-sonnet",
        (200_000, 8_192, "anthropic", true, true, true),
    ),
    (
        "claude-3-5-sonnet",
        (200_000, 8_192, "anthropic", true, true, true),
    ),
    (
        "claude-3.5-haiku",
        (200_000, 8_192, "anthropic", true, false, true),
    ),
    (
        "claude-3-5-haiku",
        (200_000, 8_192, "anthropic", true, false, true),
    ),
    (
        "claude-3-opus",
        (200_000, 4_096, "anthropic", true, true, true),
    ),
    (
        "claude-3.0-opus",
        (200_000, 4_096, "anthropic", true, true, true),
    ),
    ("claude-4", (200_000, 16_384, "anthropic", true, true, true)),
    (
        "claude-4-opus",
        (200_000, 16_384, "anthropic", true, true, true),
    ),
    (
        "claude-4-sonnet",
        (200_000, 16_384, "anthropic", true, true, true),
    ),
    // OpenAI
    ("gpt-4o", (128_000, 16_384, "openai", true, true, true)),
    ("gpt-4o-mini", (128_000, 16_384, "openai", true, true, true)),
    ("gpt-4-turbo", (128_000, 4_096, "openai", true, true, true)),
    ("gpt-4-1106", (128_000, 4_096, "openai", true, true, true)),
    ("gpt-4", (8_192, 4_096, "openai", true, false, true)),
    ("o1", (200_000, 100_000, "openai", true, true, true)),
    ("o1-mini", (128_000, 65_536, "openai", true, false, true)),
    ("o1-preview", (128_000, 32_768, "openai", true, false, true)),
    ("o3", (200_000, 100_000, "openai", true, true, true)),
    ("o3-mini", (200_000, 65_536, "openai", true, false, true)),
    ("o4-mini", (200_000, 100_000, "openai", true, true, true)),
    // Google Gemini
    (
        "gemini-2.5-pro",
        (1_000_000, 65_536, "google", true, true, true),
    ),
    (
        "gemini-2.5-flash",
        (1_000_000, 65_536, "google", true, true, true),
    ),
    (
        "gemini-2.0-flash",
        (1_000_000, 8_192, "google", true, true, true),
    ),
    (
        "gemini-1.5-pro",
        (1_000_000, 8_192, "google", true, true, true),
    ),
    (
        "gemini-1.5-flash",
        (1_000_000, 8_192, "google", true, true, true),
    ),
    // DeepSeek
    (
        "deepseek-chat",
        (64_000, 8_192, "deepseek", true, false, true),
    ),
    (
        "deepseek-coder",
        (64_000, 8_192, "deepseek", true, false, true),
    ),
    (
        "deepseek-reasoner",
        (64_000, 8_192, "deepseek", true, false, true),
    ),
    // Groq
    ("llama-3.3-70b", (128_000, 8_192, "groq", true, false, true)),
    ("llama-3.1-70b", (128_000, 8_192, "groq", true, false, true)),
    ("llama-3.1-8b", (128_000, 8_192, "groq", true, false, true)),
    ("mixtral-8x7b", (32_768, 4_096, "groq", true, false, true)),
    // Mistral
    (
        "mistral-large",
        (128_000, 8_192, "mistral", true, false, true),
    ),
    (
        "mistral-medium",
        (128_000, 8_192, "mistral", true, false, true),
    ),
    ("codestral", (256_000, 8_192, "mistral", true, false, true)),
    // xAI
    ("grok-2", (128_000, 8_192, "xai", true, true, true)),
    ("grok-3", (128_000, 16_384, "xai", true, true, true)),
    ("grok-3-mini", (128_000, 16_384, "xai", true, false, true)),
];

fn builtin_model_info(model_lower: &str) -> ModelInfo {
    // Exact match first
    if let Some((id, preset)) = BUILTIN_MODELS.iter().find(|(id, _)| *id == model_lower) {
        return preset_to_info(id, preset);
    }

    // Substring match (e.g. "anthropic/claude-3.5-sonnet-20241022" contains "claude-3.5-sonnet")
    if let Some((id, preset)) = BUILTIN_MODELS
        .iter()
        .find(|(id, _)| model_lower.contains(id))
    {
        return preset_to_info(id, preset);
    }

    // Fallback: infer provider from common prefixes
    let provider = infer_provider(model_lower);
    let context = infer_context_window(model_lower);

    ModelInfo {
        id: model_lower.to_string(),
        provider: provider.to_string(),
        context_window: context,
        ..Default::default()
    }
}

fn preset_to_info(id: &str, preset: &Preset) -> ModelInfo {
    ModelInfo {
        id: id.to_string(),
        provider: preset.2.to_string(),
        context_window: preset.0,
        max_output_tokens: preset.1,
        supports_tools: preset.3,
        supports_vision: preset.4,
        supports_streaming: preset.5,
    }
}

/// Infer provider from model id substring.
fn infer_provider(model: &str) -> &'static str {
    if model.contains("claude") {
        "anthropic"
    } else if model.contains("gpt")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        "openai"
    } else if model.contains("gemini") {
        "google"
    } else if model.contains("deepseek") {
        "deepseek"
    } else if model.contains("llama") || model.contains("mixtral") {
        "groq"
    } else if model.contains("mistral") || model.contains("codestral") {
        "mistral"
    } else if model.contains("grok") {
        "xai"
    } else {
        "unknown"
    }
}

/// Infer a reasonable context window for an unknown model variant.
fn infer_context_window(model: &str) -> u64 {
    if model.contains("claude") {
        return 200_000;
    }
    if model.contains("gemini") {
        return 1_000_000;
    }
    if model.contains("deepseek") {
        return 64_000;
    }
    if model.contains("gpt-4o") {
        return 128_000;
    }
    if model.contains("gpt-4") {
        return 8_192;
    }
    if model.contains("o1") || model.contains("o3") || model.contains("o4") {
        return 200_000;
    }
    128_000 // Safe default
}

// ── External model metadata ───────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct ExternalModelEntry {
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    provider: Option<String>,
}

static EXTERNAL_INDEX: OnceLock<Option<BTreeMap<String, ExternalModelEntry>>> = OnceLock::new();

fn external_model_info(model_lower: &str) -> Option<ModelInfo> {
    let index = EXTERNAL_INDEX.get_or_init(load_external_index).as_ref()?;

    let entry = index.get(model_lower)?;
    let builtin = builtin_model_info(model_lower);

    Some(ModelInfo {
        id: model_lower.to_string(),
        provider: entry.provider.clone().unwrap_or(builtin.provider),
        context_window: entry.context_window.unwrap_or(builtin.context_window),
        max_output_tokens: entry.max_output_tokens.unwrap_or(builtin.max_output_tokens),
        supports_tools: builtin.supports_tools,
        supports_vision: builtin.supports_vision,
        supports_streaming: builtin.supports_streaming,
    })
}

fn load_external_index() -> Option<BTreeMap<String, ExternalModelEntry>> {
    let mut candidates = Vec::new();

    if let Ok(path) = std::env::var("RUSTCODE_MODELS_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(&home).join(".cache/rustcode/models.json"));
        candidates.push(PathBuf::from(&home).join(".rustcode/models.json"));
    }
    if let Ok(cache) = std::env::var("XDG_CACHE_HOME") {
        candidates.push(PathBuf::from(cache).join("rustcode/models.json"));
    }

    for candidate in candidates {
        let Ok(raw) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<BTreeMap<String, ExternalModelEntry>>(&raw) else {
            continue;
        };
        return Some(parsed);
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_exact_match() {
        let info = model_info("claude-3.5-sonnet");
        assert_eq!(info.context_window, 200_000);
        assert_eq!(info.max_output_tokens, 8_192);
        assert_eq!(info.provider, "anthropic");
        assert!(info.supports_tools);
        assert!(info.supports_vision);
    }

    #[test]
    fn known_model_substring_match() {
        // Qualified model ids like "anthropic/claude-3.5-sonnet-20241022"
        let info = model_info("anthropic/claude-3.5-sonnet-20241022");
        assert_eq!(info.context_window, 200_000);
        assert_eq!(info.provider, "anthropic");
    }

    #[test]
    fn openai_models() {
        let info = model_info("gpt-4o");
        assert_eq!(info.context_window, 128_000);
        assert_eq!(info.provider, "openai");
        assert!(info.supports_vision);

        let info = model_info("o3-mini");
        assert_eq!(info.context_window, 200_000);
        assert_eq!(info.provider, "openai");
    }

    #[test]
    fn gemini_models() {
        let info = model_info("gemini-2.5-pro");
        assert_eq!(info.context_window, 1_000_000);
        assert_eq!(info.provider, "google");
    }

    #[test]
    fn deepseek_models() {
        let info = model_info("deepseek-chat");
        assert_eq!(info.context_window, 64_000);
        assert_eq!(info.provider, "deepseek");
    }

    #[test]
    fn unknown_model_gets_defaults() {
        let info = model_info("totally-unknown-model-v99");
        assert_eq!(info.context_window, 128_000);
        assert_eq!(info.max_output_tokens, 4_096);
        assert_eq!(info.provider, "unknown");
    }

    #[test]
    fn context_limit_convenience() {
        assert_eq!(context_limit("gpt-4o"), 128_000);
        assert_eq!(context_limit("gemini-2.0-flash"), 1_000_000);
    }

    #[test]
    fn list_models_by_provider() {
        let anthropic = list_models("anthropic");
        assert!(anthropic.contains(&"claude-3.5-sonnet"));
        assert!(anthropic.contains(&"claude-4"));

        let openai = list_models("openai");
        assert!(openai.contains(&"gpt-4o"));
        assert!(openai.contains(&"o3"));
    }

    #[test]
    fn infer_provider_from_name() {
        assert_eq!(infer_provider("claude-unknown-variant"), "anthropic");
        assert_eq!(infer_provider("gpt-5-turbo"), "openai");
        assert_eq!(infer_provider("gemini-3-ultra"), "google");
        assert_eq!(infer_provider("random-model"), "unknown");
    }

    #[test]
    fn case_insensitive_lookup() {
        let info = model_info("Claude-3.5-Sonnet");
        assert_eq!(info.context_window, 200_000);
    }
}
