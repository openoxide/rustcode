use rustcode_auth::{methods_for_provider, AuthMethod};
use rustcode_llm::connected_provider_ids;

use super::{ConnectMethod, ProviderEntry};

struct ProviderInfo {
    id: &'static str,
    display_name: &'static str,
    env_hint: Option<&'static str>,
}

const PROVIDER_CATALOG: &[ProviderInfo] = &[
    ProviderInfo {
        id: "anthropic",
        display_name: "Anthropic Claude",
        env_hint: Some("ANTHROPIC_API_KEY"),
    },
    ProviderInfo {
        id: "openai",
        display_name: "OpenAI",
        env_hint: Some("OPENAI_API_KEY"),
    },
    ProviderInfo {
        id: "google",
        display_name: "Google Gemini",
        env_hint: Some("GOOGLE_API_KEY"),
    },
    ProviderInfo {
        id: "groq",
        display_name: "Groq",
        env_hint: Some("GROQ_API_KEY"),
    },
    ProviderInfo {
        id: "xai",
        display_name: "xAI Grok",
        env_hint: Some("XAI_API_KEY"),
    },
    ProviderInfo {
        id: "mistral",
        display_name: "Mistral AI",
        env_hint: Some("MISTRAL_API_KEY"),
    },
    ProviderInfo {
        id: "deepinfra",
        display_name: "DeepInfra",
        env_hint: Some("DEEPINFRA_API_KEY"),
    },
    ProviderInfo {
        id: "cerebras",
        display_name: "Cerebras",
        env_hint: Some("CEREBRAS_API_KEY"),
    },
    ProviderInfo {
        id: "togetherai",
        display_name: "Together AI",
        env_hint: Some("TOGETHERAI_API_KEY"),
    },
    ProviderInfo {
        id: "perplexity",
        display_name: "Perplexity",
        env_hint: Some("PERPLEXITY_API_KEY"),
    },
    ProviderInfo {
        id: "openrouter",
        display_name: "OpenRouter",
        env_hint: Some("OPENROUTER_API_KEY"),
    },
    ProviderInfo {
        id: "ollama",
        display_name: "Ollama (local)",
        env_hint: None,
    },
    ProviderInfo {
        id: "github-copilot",
        display_name: "GitHub Copilot",
        env_hint: None,
    },
    ProviderInfo {
        id: "github-copilot-enterprise",
        display_name: "GitHub Copilot Enterprise",
        env_hint: None,
    },
    ProviderInfo {
        id: "vercel",
        display_name: "Vercel AI Gateway",
        env_hint: Some("VERCEL_API_KEY"),
    },
    ProviderInfo {
        id: "azure",
        display_name: "Azure OpenAI",
        env_hint: Some("AZURE_OPENAI_API_KEY"),
    },
    ProviderInfo {
        id: "gitlab",
        display_name: "GitLab",
        env_hint: Some("GITLAB_API_KEY"),
    },
];

/// Build the full provider list with connected status.
///
/// Connected providers (those with a credential available) are sorted first.
pub(super) fn build_provider_entries() -> Vec<ProviderEntry> {
    let connected_ids = connected_provider_ids();
    let connected_set: std::collections::HashSet<&str> =
        connected_ids.iter().map(String::as_str).collect();

    let mut entries: Vec<ProviderEntry> = PROVIDER_CATALOG
        .iter()
        .map(|info| ProviderEntry {
            provider_id: info.id.to_string(),
            display_name: info.display_name.to_string(),
            connected: connected_set.contains(info.id),
        })
        .collect();

    // Connected providers first, then alphabetical by display name.
    entries.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    entries
}

/// Filter provider entries by case-insensitive substring match on id or display name.
pub(super) fn filter_provider_entries(entries: &[ProviderEntry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    let needle = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.provider_id.to_ascii_lowercase().contains(&needle)
                || e.display_name.to_ascii_lowercase().contains(&needle)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Return the human-readable display name for a provider.
#[must_use]
pub(super) fn provider_display_name(provider_id: &str) -> String {
    PROVIDER_CATALOG
        .iter()
        .find(|info| info.id == provider_id)
        .map(|info| info.display_name.to_string())
        .unwrap_or_else(|| provider_id.to_string())
}

/// Return the primary environment variable hint for a provider's API key, if any.
#[must_use]
pub(super) fn provider_env_hint(provider_id: &str) -> Option<String> {
    PROVIDER_CATALOG
        .iter()
        .find(|info| info.id == provider_id)
        .and_then(|info| info.env_hint)
        .map(ToOwned::to_owned)
}

/// Build the list of connect methods for a provider.
///
/// Always includes `ApiKey`. Includes `OAuthDeviceCode` only for providers that
/// support it. Includes `Disconnect` when the provider currently has credentials.
pub(super) fn provider_connect_methods(provider_id: &str, connected: bool) -> Vec<ConnectMethod> {
    let mut methods = Vec::new();
    let auth_methods = methods_for_provider(provider_id);
    for method in &auth_methods {
        match method {
            AuthMethod::OAuthDeviceCode => methods.push(ConnectMethod::OAuthDeviceCode),
            AuthMethod::ApiKey => methods.push(ConnectMethod::ApiKey),
            // Skip browser OAuth — requires an external browser + callback server.
            AuthMethod::OAuthBrowser => {}
        }
    }
    if connected {
        methods.push(ConnectMethod::Disconnect);
    }
    methods
}
