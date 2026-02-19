mod clients;
mod endpoints;
pub mod model_registry;
mod provider;
mod provider_presets;
mod streaming;
mod transforms;
mod types;

use std::sync::Arc;

use clients::{
    AnthropicClient, GoogleGenerativeAiClient, OpenAiCompatibleClient, VercelAiGatewayClient,
};
use provider::{
    missing_api_key_message, missing_base_url_message, resolve_provider, ProviderProtocol,
};
use rustcode_core::config::ResolvedConfig;

#[cfg(test)]
use reqwest::header::HeaderMap;
#[cfg(test)]
use serde_json::json;

pub use model_registry::ModelInfo;
pub use provider::{
    builtin_provider_ids, derive_copilot_enterprise_base_url, diagnose_provider, ApiKeySource,
    ProviderDiagnostics, ProviderProtocolName,
};
pub use types::{
    ChatMessage, ChatRequest, ChatResponse, ChatRole, LlmClient, LlmError, LlmRequest,
    LlmResponse, NullLlmClient, RequestInitiator, ToolCall, ToolSpec, TokenUsage,
};

pub fn build_client(config: &ResolvedConfig) -> Result<Arc<dyn LlmClient>, LlmError> {
    let provider = resolve_provider(config)?;
    match provider.protocol {
        ProviderProtocol::Null => Ok(Arc::new(NullLlmClient)),
        ProviderProtocol::OpenAiCompatible => {
            require_network(config)?;

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
            require_network(config)?;

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
            require_network(config)?;

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
        ProviderProtocol::GoogleGenerativeAi => {
            require_network(config)?;

            let base_url = provider
                .base_url
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
            let api_key = provider.api_key.ok_or_else(|| {
                LlmError::Config(missing_api_key_message(
                    &provider.provider_id,
                    &provider.api_key_env_candidates,
                ))
            })?;
            Ok(Arc::new(GoogleGenerativeAiClient::new(
                provider.provider_id,
                base_url,
                api_key,
            )?))
        }
    }
}

fn require_network(config: &ResolvedConfig) -> Result<(), LlmError> {
    if config.allow_network {
        Ok(())
    } else {
        Err(LlmError::Config(
            "network access is disabled; set allow_network=true to use remote LLM providers"
                .to_string(),
        ))
    }
}

#[cfg(test)]
use endpoints::{
    apply_provider_default_headers, normalize_anthropic_messages_endpoint,
    normalize_google_base_url, normalize_google_generate_endpoint, normalize_openai_chat_endpoint,
    normalize_vercel_gateway_endpoint,
};
#[cfg(test)]
use provider::model_for_provider;
#[cfg(test)]
use streaming::{extract_stream_error_message, parse_sse_data_block, take_next_sse_block};
#[cfg(test)]
use transforms::{
    error_http_status, extract_anthropic_stream_delta, extract_anthropic_text,
    extract_anthropic_tool_calls, extract_anthropic_usage, extract_error_message,
    extract_gateway_stream_delta, extract_google_text, extract_google_tool_calls,
    extract_google_usage, extract_openai_stream_delta, extract_openai_text,
    extract_openai_tool_calls, extract_openai_usage, is_content_filter_error,
    sanitize_google_schema,
};

#[cfg(test)]
mod tests;
