use super::*;

#[test]
fn resolves_provider_from_model_prefix_when_provider_is_null() {
    let _auth = AuthFileGuard::empty("llm-model-prefix-null-provider");
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
fn vercel_provider_resolves_as_gateway_protocol_and_requires_key() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
    let _auth = AuthFileGuard::empty("llm-vercel-policy");
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
            Some(1_234_567_890),
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
    let _auth = AuthFileGuard::empty("llm-diagnostics-missing-key");
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
    let _auth = AuthFileGuard::empty("llm-policy-select");
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
    let _auth = AuthFileGuard::empty("llm-policy-diagnostics");
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
