use super::*;

#[test]
fn auth_login_without_provider_lists_models_and_methods() {
    let models_path = make_temp_file_path("auth-login-models");
    std::fs::write(
        &models_path,
        r#"{
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } },
  "openrouter": { "name": "OpenRouter", "models": { "openai/gpt-5": {} } },
  "anthropic": { "name": "Anthropic", "models": { "claude-sonnet": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["auth", "login"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode auth login");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("usage_api_key=rustcode auth login <provider> --from-env <ENV_VAR>"));
    assert!(stdout.contains(
        "usage_oauth=rustcode auth login <provider> --method <oauth_device_code|oauth_browser>"
    ));
    assert!(stdout.contains("provider=anthropic\tname=Anthropic\tmethods=api_key"));
    assert!(stdout
        .contains("provider=openai\tname=OpenAI\tmethods=oauth_device_code|oauth_browser|api_key"));
    assert!(stdout.contains("provider=openrouter\tname=OpenRouter\tmethods=api_key"));
}

#[test]
fn auth_login_without_provider_json_lists_models_and_methods() {
    let models_path = make_temp_file_path("auth-login-json-models");
    std::fs::write(
        &models_path,
        r#"{
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } },
  "openrouter": { "name": "OpenRouter", "models": { "openai/gpt-5": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "auth", "login"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode auth login");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(
        payload["usage"]["api_key"].as_str(),
        Some("rustcode auth login <provider> --from-env <ENV_VAR>")
    );
    let providers = payload["providers"]
        .as_array()
        .expect("providers should be array");
    assert!(providers
        .iter()
        .any(|row| row["id"].as_str() == Some("openai")));
    assert!(providers
        .iter()
        .any(|row| row["id"].as_str() == Some("openrouter")));
}

#[test]
fn auth_login_from_env_stores_key() {
    let auth_path = make_temp_file_path("auth-login-set-key");

    let login_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-login-secret")
        .output()
        .expect("must run rustcode auth login from env");

    assert!(
        login_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&login_output.stdout),
        String::from_utf8_lossy(&login_output.stderr)
    );
    let login_stdout = String::from_utf8(login_output.stdout).expect("stdout must be utf8");
    assert!(login_stdout.contains("stored api key for provider=openrouter"));

    let status_output = Command::new(rustcode_bin())
        .args(["auth", "status", "openrouter"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth status");

    assert!(
        status_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status_stdout = String::from_utf8(status_output.stdout).expect("stdout must be utf8");
    assert!(status_stdout.contains("credential=stored:api_key"));
}

#[test]
fn auth_login_from_env_json_emits_authorized_stage() {
    let auth_path = make_temp_file_path("auth-login-json-set-key");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "auth",
            "login",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-login-secret")
        .output()
        .expect("must run rustcode auth login from env");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["provider"].as_str(), Some("openrouter"));
    assert_eq!(payload["method"].as_str(), Some("api_key"));
    assert_eq!(payload["stage"].as_str(), Some("authorized"));
    assert_eq!(payload["source"].as_str(), Some("env"));
    assert_eq!(payload["credential"].as_str(), Some("stored:api_key"));
}

#[test]
fn auth_login_from_env_defaults_to_api_key_for_openai() {
    let auth_path = make_temp_file_path("auth-login-openai-set-key");

    let login_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "openai",
            "--from-env",
            "RUSTCODE_OPENAI_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_OPENAI_KEY", "openai-integration-secret")
        .output()
        .expect("must run rustcode auth login from env");

    assert!(
        login_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&login_output.stdout),
        String::from_utf8_lossy(&login_output.stderr)
    );
    let login_stdout = String::from_utf8(login_output.stdout).expect("stdout must be utf8");
    assert!(login_stdout.contains("stored api key for provider=openai"));

    let status_output = Command::new(rustcode_bin())
        .args(["auth", "status", "openai"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth status");

    assert!(
        status_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status_stdout = String::from_utf8(status_output.stdout).expect("stdout must be utf8");
    assert!(status_stdout.contains("credential=stored:api_key"));
}

#[test]
fn auth_login_from_env_requires_provider() {
    let output = Command::new(rustcode_bin())
        .args(["auth", "login", "--from-env", "RUSTCODE_TEST_KEY"])
        .output()
        .expect("must run rustcode auth login");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("`--from-env` requires a provider"));
}

#[test]
fn auth_login_from_env_requires_provider_json_emits_failed_envelope() {
    let output = Command::new(rustcode_bin())
        .args(["--json", "auth", "login", "--from-env", "RUSTCODE_TEST_KEY"])
        .output()
        .expect("must run rustcode auth login");

    assert!(!output.status.success(), "command should fail");

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["command"].as_str(), Some("auth.login"));
    assert_eq!(payload["stage"].as_str(), Some("failed"));
    assert_eq!(payload["error_kind"].as_str(), Some("validation"));

    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("`--from-env` requires a provider"));
}

#[test]
fn auth_login_api_key_provider_requires_from_env_in_non_interactive_mode() {
    let output = Command::new(rustcode_bin())
        .args(["auth", "login", "openrouter"])
        .output()
        .expect("must run rustcode auth login");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("requires --from-env in non-interactive mode"));
}

#[test]
fn auth_methods_gitlab_reports_browser_oauth() {
    let output = Command::new(rustcode_bin())
        .args(["auth", "methods", "gitlab"])
        .output()
        .expect("must run rustcode auth methods");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("methods=oauth_browser, api_key"));
}

#[test]
fn auth_methods_without_provider_lists_available_rows() {
    let models_path = make_temp_file_path("auth-methods-models");
    std::fs::write(
        &models_path,
        r#"{
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } },
  "openrouter": { "name": "OpenRouter", "models": { "openai/gpt-5": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["auth", "methods"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode auth methods");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("providers=2"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "provider=openai\tname=OpenAI\tmethods=oauth_device_code|oauth_browser|api_key"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("provider=openrouter\tname=OpenRouter\tmethods=api_key"),
        "stdout: {stdout}"
    );
}

#[test]
fn auth_methods_json_provider_detail_is_parseable() {
    let output = Command::new(rustcode_bin())
        .args(["--json", "auth", "methods", "openai"])
        .output()
        .expect("must run rustcode auth methods");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["provider"].as_str(), Some("openai"));

    let methods = payload["methods"]
        .as_array()
        .expect("methods should be array")
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert!(methods.contains(&"oauth_device_code"));
    assert!(methods.contains(&"oauth_browser"));
    assert!(methods.contains(&"api_key"));
}

#[test]
fn auth_methods_json_without_provider_lists_available_rows() {
    let models_path = make_temp_file_path("auth-methods-json-models");
    std::fs::write(
        &models_path,
        r#"{
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } },
  "openrouter": { "name": "OpenRouter", "models": { "openai/gpt-5": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "auth", "methods"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode auth methods");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert!(payload["warning"].is_null());

    let providers = payload["providers"]
        .as_array()
        .expect("providers should be array");
    assert_eq!(providers.len(), 2);

    let openai = providers
        .iter()
        .find(|row| row["id"].as_str() == Some("openai"))
        .expect("openai row should exist");
    assert_eq!(openai["name"].as_str(), Some("OpenAI"));
    assert!(openai["methods"]
        .as_array()
        .expect("methods should be array")
        .iter()
        .any(|value| value.as_str() == Some("oauth_browser")));

    let openrouter = providers
        .iter()
        .find(|row| row["id"].as_str() == Some("openrouter"))
        .expect("openrouter row should exist");
    assert_eq!(openrouter["name"].as_str(), Some("OpenRouter"));
    assert_eq!(
        openrouter["methods"]
            .as_array()
            .expect("methods should be array")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec!["api_key"]
    );
}

#[test]
fn auth_login_gitlab_dot_com_browser_no_wait_uses_bundled_client_id() {
    let output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "gitlab",
            "--method",
            "oauth_browser",
            "--oauth-port",
            "19081",
            "--no-wait",
        ])
        .output()
        .expect("must run rustcode auth login gitlab");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("provider=gitlab"));
    assert!(stdout.contains("authorize_url=https://gitlab.com/oauth/authorize"));
    assert!(stdout.contains("status=awaiting_browser_callback"));
}
