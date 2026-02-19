use crate::provider::ProviderProtocol;
use crate::types::RequestInitiator;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

pub(crate) fn apply_provider_default_headers(
    provider_id: &str,
    headers: &mut HeaderMap,
    initiator: RequestInitiator,
) -> Result<(), String> {
    match provider_id {
        "openrouter" | "vercel" => {
            headers.insert(
                HeaderName::from_static("http-referer"),
                HeaderValue::from_static("https://opencode.ai/"),
            );
            headers.insert(
                HeaderName::from_static("x-title"),
                HeaderValue::from_static("opencode"),
            );
        }
        "github-copilot" | "github-copilot-enterprise" => {
            headers.insert(
                HeaderName::from_static("openai-intent"),
                HeaderValue::from_static("conversation-edits"),
            );
            headers.insert(
                HeaderName::from_static("x-initiator"),
                HeaderValue::from_static(match initiator {
                    RequestInitiator::User => "user",
                    RequestInitiator::Agent => "agent",
                }),
            );
            let user_agent = format!("rustcode/{}", env!("CARGO_PKG_VERSION"));
            let value = HeaderValue::from_str(&user_agent)
                .map_err(|err| format!("invalid user-agent header: {err}"))?;
            headers.insert(HeaderName::from_static("user-agent"), value);
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn normalize_openai_chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

pub(crate) fn normalize_copilot_chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

pub(crate) fn normalize_anthropic_messages_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/messages")
    } else {
        format!("{trimmed}/v1/messages")
    }
}

pub(crate) fn normalize_vercel_gateway_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/language-model") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/language-model")
    }
}

pub(crate) fn normalize_google_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1beta") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1beta")
    }
}

pub(crate) fn normalize_google_generate_endpoint(base_url: &str) -> String {
    let normalized = normalize_google_base_url(base_url);
    format!("{normalized}/models/*:generateContent")
}

pub(crate) fn normalize_endpoint(protocol: ProviderProtocol, base_url: &str) -> String {
    match protocol {
        ProviderProtocol::Null => base_url.to_string(),
        ProviderProtocol::OpenAiCompatible => normalize_openai_chat_endpoint(base_url),
        ProviderProtocol::AnthropicMessages => normalize_anthropic_messages_endpoint(base_url),
        ProviderProtocol::VercelAiGateway => normalize_vercel_gateway_endpoint(base_url),
        ProviderProtocol::GoogleGenerativeAi => normalize_google_generate_endpoint(base_url),
    }
}

pub(crate) fn normalize_endpoint_for_provider(
    protocol: ProviderProtocol,
    base_url: &str,
    provider_id: &str,
) -> String {
    if matches!(provider_id, "github-copilot" | "github-copilot-enterprise")
        && matches!(protocol, ProviderProtocol::OpenAiCompatible)
    {
        return normalize_copilot_chat_endpoint(base_url);
    }
    normalize_endpoint(protocol, base_url)
}

pub(crate) fn truncate_for_error(body: &str) -> String {
    // Prefer a human-readable "message" field from JSON error bodies so we
    // never embed raw JSON in user-visible error strings.
    //
    // Handles the most common provider formats:
    //   OpenAI / OpenRouter: {"error": {"message": "...", "type": "...", ...}}
    //   Anthropic:           {"error": {"message": "...", "type": "..."}}
    //   Generic:             {"message": "..."}
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        // Nested: {"error": {"message": "..."}}
        if let Some(msg) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return msg.to_string();
        }
        // Top-level: {"message": "..."}
        if let Some(msg) = v
            .get("message")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
        {
            return msg.to_string();
        }
    }
    // Fallback: plain-text body, truncated
    const LIMIT: usize = 200;
    if body.chars().count() <= LIMIT {
        body.to_string()
    } else {
        format!("{}…", body.chars().take(LIMIT).collect::<String>())
    }
}
