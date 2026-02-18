use super::*;

#[test]
fn github_copilot_provider_has_default_base_url() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
    std::env::set_var("GITHUB_COPILOT_TOKEN", "copilot-test-token");

    let cfg = ResolvedConfig {
        allow_network: true,
        llm_provider: "github-copilot".to_string(),
        ..ResolvedConfig::default()
    };

    let provider = resolve_provider(&cfg).expect("provider must resolve");
    assert_eq!(provider.provider_id, "github-copilot");
    assert_eq!(provider.protocol, ProviderProtocol::OpenAiCompatible);
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://api.githubcopilot.com")
    );
    assert_eq!(provider.api_key.as_deref(), Some("copilot-test-token"));

    std::env::remove_var("GITHUB_COPILOT_TOKEN");
}

#[test]
fn github_copilot_headers_include_required_defaults() {
    let mut headers = HeaderMap::new();
    apply_provider_default_headers("github-copilot", &mut headers, RequestInitiator::User)
        .expect("headers should apply");

    assert_eq!(
        headers.get("openai-intent").and_then(|v| v.to_str().ok()),
        Some("conversation-edits")
    );
    assert_eq!(
        headers.get("x-initiator").and_then(|v| v.to_str().ok()),
        Some("user")
    );
    assert!(headers.contains_key("user-agent"));
}

#[test]
fn github_copilot_headers_use_agent_initiator_when_requested() {
    let mut headers = HeaderMap::new();
    apply_provider_default_headers("github-copilot", &mut headers, RequestInitiator::Agent)
        .expect("headers should apply");

    assert_eq!(
        headers.get("x-initiator").and_then(|v| v.to_str().ok()),
        Some("agent")
    );
}

#[test]
fn github_copilot_enterprise_uses_same_defaults() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
    std::env::set_var("GITHUB_COPILOT_TOKEN", "enterprise-token");

    let cfg = ResolvedConfig {
        allow_network: true,
        llm_provider: "github-copilot-enterprise".to_string(),
        ..ResolvedConfig::default()
    };

    let provider = resolve_provider(&cfg).expect("provider must resolve");
    assert_eq!(provider.provider_id, "github-copilot-enterprise");
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://api.githubcopilot.com")
    );

    std::env::remove_var("GITHUB_COPILOT_TOKEN");
}

#[test]
fn github_copilot_endpoint_uses_chat_completions_without_v1_prefix() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
    std::env::set_var("GITHUB_COPILOT_TOKEN", "copilot-test-token");

    let cfg = ResolvedConfig {
        allow_network: true,
        llm_provider: "github-copilot".to_string(),
        ..ResolvedConfig::default()
    };

    let diag = diagnose_provider(&cfg, Some("github-copilot")).expect("diagnostic should resolve");
    assert_eq!(
        diag.endpoint.as_deref(),
        Some("https://api.githubcopilot.com/chat/completions")
    );

    std::env::remove_var("GITHUB_COPILOT_TOKEN");
}

#[test]
fn github_copilot_enterprise_base_url_can_be_derived_from_env_domain() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");
    std::env::set_var("GITHUB_COPILOT_TOKEN", "copilot-test-token");
    std::env::set_var("GITHUB_COPILOT_ENTERPRISE_DOMAIN", "github.example.com");

    let cfg = ResolvedConfig {
        allow_network: true,
        llm_provider: "github-copilot-enterprise".to_string(),
        ..ResolvedConfig::default()
    };

    let diag = diagnose_provider(&cfg, Some("github-copilot-enterprise"))
        .expect("diagnostic should resolve");
    assert_eq!(
        diag.base_url.as_deref(),
        Some("https://copilot-api.github.example.com")
    );

    std::env::remove_var("GITHUB_COPILOT_ENTERPRISE_DOMAIN");
    std::env::remove_var("GITHUB_COPILOT_TOKEN");
}

#[test]
fn copilot_enterprise_domain_derivation_handles_prefixed_and_url_values() {
    assert_eq!(
        derive_copilot_enterprise_base_url("copilot-api.github.example.com"),
        "https://copilot-api.github.example.com"
    );
    assert_eq!(
        derive_copilot_enterprise_base_url("https://copilot-api.github.example.com/"),
        "https://copilot-api.github.example.com"
    );
}

#[test]
fn github_copilot_enterprise_base_url_can_be_derived_from_auth_store_domain() {
    let _guard = ENV_MUTEX.lock().expect("env mutex must lock");

    let auth_path = make_temp_file_path("copilot-enterprise-auth-domain");
    std::fs::write(
        &auth_path,
        r#"{
  "providers": {
"github-copilot-enterprise": {
  "type": "api_key",
  "key": "enterprise-token",
  "domain": "github.example.com"
}
  }
}"#,
    )
    .expect("must write auth fixture");

    std::env::set_var("RUSTCODE_AUTH_FILE", auth_path.as_os_str());
    std::env::remove_var("GITHUB_COPILOT_ENTERPRISE_DOMAIN");
    std::env::remove_var("GITHUB_COPILOT_ENTERPRISE_BASE_URL");
    std::env::remove_var("GITHUB_COPILOT_TOKEN");

    let cfg = ResolvedConfig {
        allow_network: true,
        llm_provider: "github-copilot-enterprise".to_string(),
        ..ResolvedConfig::default()
    };

    let diag = diagnose_provider(&cfg, Some("github-copilot-enterprise"))
        .expect("diagnostic should resolve");
    assert_eq!(
        diag.base_url.as_deref(),
        Some("https://copilot-api.github.example.com")
    );

    std::env::remove_var("RUSTCODE_AUTH_FILE");
    let _ = std::fs::remove_file(&auth_path);
}
