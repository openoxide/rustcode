use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

fn rustcode_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rustcode-cli")
}

fn make_temp_file_path(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("rustcode-cli-{name}-{pid}-{now}.json"))
}

fn http_request(port: u16, path: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

#[test]
fn json_stream_includes_schema_version_and_completion_event() {
    let output = Command::new(rustcode_bin())
        .args(["--json", "run", "integration-stream"])
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() >= 2, "expected streamed event lines");

    let mut saw_completed = false;
    for line in lines {
        let value: Value = serde_json::from_str(line).expect("line must be valid json");
        assert_eq!(value["schema_version"].as_u64(), Some(1));

        if value["payload"]["type"].as_str() == Some("Completed") {
            saw_completed = true;
        }
    }

    assert!(saw_completed, "expected Completed event in stream");
}

#[test]
fn human_run_output_is_plain_text_by_default() {
    let output = Command::new(rustcode_bin())
        .args(["run", "integration-human"])
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("null-llm response"));
    assert!(
        !stdout.contains("OutputChunk"),
        "stdout should be coalesced"
    );
}

#[test]
fn human_run_output_uses_event_envelope_with_event_debug() {
    let output = Command::new(rustcode_bin())
        .args(["--event-debug", "run", "integration-human-debug"])
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("OutputChunk"));
    assert!(stdout.contains("Completed"));
}

#[test]
fn auth_set_key_and_status_round_trip() {
    let auth_path = make_temp_file_path("auth-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");

    assert!(
        set_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&set_output.stdout),
        String::from_utf8_lossy(&set_output.stderr)
    );

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
    assert!(status_stdout.contains("provider=openrouter"));
    assert!(status_stdout.contains("credential=stored:api_key"));
}

#[test]
fn auth_status_json_is_parseable() {
    let auth_path = make_temp_file_path("auth-status-json-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");
    assert!(set_output.status.success(), "set-key should succeed");

    let status_output = Command::new(rustcode_bin())
        .args(["--json", "auth", "status", "openrouter"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth status");

    assert!(
        status_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );

    let stdout = String::from_utf8(status_output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["provider"].as_str(), Some("openrouter"));
    assert_eq!(payload["credential"].as_str(), Some("stored:api_key"));
    assert!(payload["methods"]
        .as_array()
        .expect("methods should be array")
        .iter()
        .any(|value| value.as_str() == Some("api_key")));
}

#[test]
fn auth_set_key_json_response_is_parseable() {
    let auth_path = make_temp_file_path("auth-set-key-json-store");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");

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
    assert_eq!(payload["action"].as_str(), Some("set_key"));
    assert_eq!(payload["credential"].as_str(), Some("stored:api_key"));
}

#[test]
fn auth_set_oauth_and_status_round_trip() {
    let auth_path = make_temp_file_path("auth-oauth-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-oauth",
            "openai",
            "--access-env",
            "RUSTCODE_TEST_ACCESS",
            "--refresh-env",
            "RUSTCODE_TEST_REFRESH",
            "--expires-unix",
            "1234567890",
            "--account-id",
            "acct_123",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_ACCESS", "oauth-access-secret")
        .env("RUSTCODE_TEST_REFRESH", "oauth-refresh-secret")
        .output()
        .expect("must run rustcode auth set-oauth");

    assert!(
        set_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&set_output.stdout),
        String::from_utf8_lossy(&set_output.stderr)
    );

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
    assert!(status_stdout.contains("credential=stored:oauth"));
}

#[test]
fn auth_set_oauth_json_response_is_parseable() {
    let auth_path = make_temp_file_path("auth-set-oauth-json-store");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "auth",
            "set-oauth",
            "openai",
            "--access-env",
            "RUSTCODE_TEST_ACCESS",
            "--refresh-env",
            "RUSTCODE_TEST_REFRESH",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_ACCESS", "oauth-access-secret")
        .env("RUSTCODE_TEST_REFRESH", "oauth-refresh-secret")
        .output()
        .expect("must run rustcode auth set-oauth");

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
    assert_eq!(payload["action"].as_str(), Some("set_oauth"));
    assert_eq!(payload["credential"].as_str(), Some("stored:oauth"));
}

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
fn auth_list_json_includes_stored_provider_rows() {
    let auth_path = make_temp_file_path("auth-list-json-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");
    assert!(set_output.status.success(), "set-key should succeed");

    let list_output = Command::new(rustcode_bin())
        .args(["--json", "auth", "list"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth list");

    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );

    let stdout = String::from_utf8(list_output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    let providers = payload["providers"]
        .as_array()
        .expect("providers should be array");
    assert_eq!(providers.len(), 1);

    let row = providers.first().expect("provider row should exist");
    assert_eq!(row["id"].as_str(), Some("openrouter"));
    assert_eq!(row["credential"].as_str(), Some("stored:api_key"));
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
fn auth_login_gitlab_browser_no_wait_emits_authorize_url() {
    let output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "gitlab",
            "--method",
            "oauth_browser",
            "--domain",
            "gitlab.example.com",
            "--oauth-port",
            "19080",
            "--no-wait",
        ])
        .env("GITLAB_OAUTH_CLIENT_ID", "gitlab-client-id")
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
    assert!(stdout.contains("method=oauth_browser"));
    assert!(stdout.contains("authorize_url=https://gitlab.example.com/oauth/authorize"));
    assert!(stdout.contains("redirect_uri=http://127.0.0.1:19080/callback"));
    assert!(stdout.contains("status=awaiting_browser_callback"));
}

#[test]
fn auth_login_gitlab_browser_requires_client_id() {
    let output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "gitlab",
            "--method",
            "oauth_browser",
            "--domain",
            "gitlab.example.com",
            "--no-wait",
        ])
        .output()
        .expect("must run rustcode auth login gitlab");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(
        stderr.contains("GITLAB_OAUTH_CLIENT_ID"),
        "stderr: {stderr}"
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

#[test]
fn auth_login_openai_browser_no_wait_emits_authorize_url() {
    let output = Command::new(rustcode_bin())
        .args([
            "auth",
            "login",
            "openai",
            "--method",
            "oauth_browser",
            "--oauth-port",
            "19455",
            "--no-wait",
        ])
        .output()
        .expect("must run rustcode auth login openai");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("provider=openai"));
    assert!(stdout.contains("method=oauth_browser"));
    assert!(stdout.contains("authorize_url=https://auth.openai.com/oauth/authorize"));
    assert!(stdout.contains("redirect_uri=http://127.0.0.1:19455/auth/callback"));
    assert!(stdout.contains("status=awaiting_browser_callback"));
}

#[test]
fn auth_login_openai_browser_no_wait_json_emits_stage_sequence() {
    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "auth",
            "login",
            "openai",
            "--method",
            "oauth_browser",
            "--oauth-port",
            "19456",
            "--no-wait",
        ])
        .output()
        .expect("must run rustcode auth login openai");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");

    let challenge: Value = serde_json::from_str(lines[0]).expect("challenge must parse");
    assert_eq!(challenge["schema_version"].as_u64(), Some(1));
    assert_eq!(challenge["provider"].as_str(), Some("openai"));
    assert_eq!(challenge["method"].as_str(), Some("oauth_browser"));
    assert_eq!(challenge["stage"].as_str(), Some("challenge"));
    assert!(challenge["authorize_url"]
        .as_str()
        .expect("authorize_url should be string")
        .contains("https://auth.openai.com/oauth/authorize"));

    let awaiting: Value = serde_json::from_str(lines[1]).expect("awaiting must parse");
    assert_eq!(awaiting["schema_version"].as_u64(), Some(1));
    assert_eq!(awaiting["provider"].as_str(), Some("openai"));
    assert_eq!(awaiting["method"].as_str(), Some("oauth_browser"));
    assert_eq!(
        awaiting["stage"].as_str(),
        Some("awaiting_browser_callback")
    );
}

#[test]
fn auth_list_and_logout_alias_work() {
    let auth_path = make_temp_file_path("auth-list-logout");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");
    assert!(
        set_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&set_output.stdout),
        String::from_utf8_lossy(&set_output.stderr)
    );

    let list_output = Command::new(rustcode_bin())
        .args(["auth", "ls"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth ls");
    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let list_stdout = String::from_utf8(list_output.stdout).expect("stdout must be utf8");
    assert!(list_stdout.contains("providers=1"));
    assert!(list_stdout.contains("provider=openrouter\tcredential=stored:api_key"));

    let logout_output = Command::new(rustcode_bin())
        .args(["auth", "logout", "openrouter"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth logout");
    assert!(
        logout_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&logout_output.stdout),
        String::from_utf8_lossy(&logout_output.stderr)
    );
    let logout_stdout = String::from_utf8(logout_output.stdout).expect("stdout must be utf8");
    assert!(logout_stdout.contains("removed credential for provider=openrouter"));
}

#[test]
fn auth_remove_json_response_is_parseable() {
    let auth_path = make_temp_file_path("auth-remove-json-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");
    assert!(set_output.status.success(), "set-key should succeed");

    let remove_output = Command::new(rustcode_bin())
        .args(["--json", "auth", "remove", "openrouter"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth remove");

    assert!(
        remove_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&remove_output.stdout),
        String::from_utf8_lossy(&remove_output.stderr)
    );

    let stdout = String::from_utf8(remove_output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("must parse json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["provider"].as_str(), Some("openrouter"));
    assert_eq!(payload["action"].as_str(), Some("remove"));
    assert_eq!(payload["removed"].as_bool(), Some(true));
}

#[test]
fn models_command_reads_custom_models_index() {
    let models_path = make_temp_file_path("models-index");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {}, "m2": {} } },
  "beta": { "name": "Beta Provider", "models": { "x1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["models", "alpha"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("alpha/m1"), "stdout: {stdout}");
    assert!(stdout.contains("alpha/m2"), "stdout: {stdout}");
}

#[test]
fn models_summary_includes_diagnostics_columns() {
    let models_path = make_temp_file_path("models-summary");
    std::fs::write(
        &models_path,
        r#"{
  "openrouter": { "name": "OpenRouter", "models": { "m1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["models"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("protocol="), "stdout: {stdout}");
    assert!(stdout.contains("api_key_source="), "stdout: {stdout}");
    assert!(stdout.contains("missing="), "stdout: {stdout}");
    assert!(stdout.contains("policy_score="), "stdout: {stdout}");
    assert!(stdout.contains("policy_selected="), "stdout: {stdout}");
    assert!(stdout.contains("policy_available="), "stdout: {stdout}");
}

#[test]
fn models_json_summary_is_parseable() {
    let models_path = make_temp_file_path("models-json-summary");
    std::fs::write(
        &models_path,
        r#"{
  "openrouter": { "name": "OpenRouter", "models": { "m1": {} } },
  "openai": { "name": "OpenAI", "models": { "gpt-5": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models --json");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("stdout must be valid json");
    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert_eq!(parsed["providers"].as_array().map(Vec::len), Some(2));
    assert!(parsed["providers"]
        .as_array()
        .expect("providers must be array")
        .iter()
        .all(|provider| provider.get("policy_score").is_some()));
}

#[test]
fn models_json_provider_detail_includes_models_array() {
    let models_path = make_temp_file_path("models-json-provider");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {}, "m2": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "models", "alpha"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models alpha --json");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let parsed: Value = serde_json::from_str(stdout.trim()).expect("stdout must be valid json");
    assert_eq!(parsed["schema_version"].as_u64(), Some(1));
    assert_eq!(parsed["provider"]["id"].as_str(), Some("alpha"));
    assert_eq!(
        parsed["provider"]["models"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(parsed["provider"]["policy_score"].is_null());
    assert_eq!(parsed["provider"]["policy_selected"].as_bool(), Some(false));
    assert!(parsed["provider"]["models"]
        .as_array()
        .expect("models must be array")
        .iter()
        .any(|item| item.as_str() == Some("alpha/m1")));
}

#[cfg(unix)]
#[test]
fn sigint_cancels_long_running_command_gracefully() {
    let mut child = Command::new(rustcode_bin())
        .args(["exec", "sleep", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("must spawn rustcode process");

    // Avoid signal race by ensuring process remains alive briefly before SIGINT.
    let mut saw_running = false;
    for _ in 0..20 {
        match child.try_wait().expect("must query child state") {
            None => {
                saw_running = true;
                break;
            }
            Some(status) => {
                panic!("child exited before signal with status: {status}");
            }
        }
    }
    assert!(saw_running, "child never reached running state");
    thread::sleep(Duration::from_millis(1000));

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("must invoke kill");
    assert!(kill_status.success(), "failed to send SIGINT");

    let output = child
        .wait_with_output()
        .expect("must collect rustcode output");

    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("execution cancelled"));
}

#[test]
fn tui_command_routes_to_tui_consumer_loop() {
    let output = Command::new(rustcode_bin())
        .arg("tui")
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(
        stdout.trim().is_empty(),
        "tui mode should not print CLI renderer output"
    );
}

#[cfg(unix)]
#[test]
fn serve_exposes_health_response_and_cancels_cleanly() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            // Some sandboxed environments disallow local TCP binds.
            return;
        }
        Err(err) => panic!("must reserve port: {err}"),
    };
    let port = listener.local_addr().expect("must read local addr").port();
    drop(listener);

    let listen = format!("127.0.0.1:{port}");
    let child = Command::new(rustcode_bin())
        .args(["serve", "--listen", &listen])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("must spawn rustcode serve process");

    let mut health_response = String::new();
    let mut connected = false;
    for _ in 0..30 {
        match http_request(port, "/health") {
            Ok(response) => {
                health_response = response;
                connected = true;
                break;
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }

    assert!(connected, "failed to connect to serve endpoint");
    assert!(
        health_response.contains("200 OK"),
        "response: {health_response}"
    );
    assert!(
        health_response.contains("{\"ok\":true}"),
        "response: {health_response}"
    );

    let not_found_response = http_request(port, "/does-not-exist").expect("must read 404 response");
    assert!(
        not_found_response.contains("404 Not Found"),
        "response: {not_found_response}"
    );
    assert!(
        not_found_response.contains("{\"error\":\"not found\"}"),
        "response: {not_found_response}"
    );

    let mut timeout_stream =
        TcpStream::connect(("127.0.0.1", port)).expect("must open timeout probe connection");
    timeout_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("must set timeout");
    let mut timeout_response = String::new();
    timeout_stream
        .read_to_string(&mut timeout_response)
        .expect("must read timeout response");
    assert!(
        timeout_response.contains("408 Request Timeout"),
        "response: {timeout_response}"
    );
    assert!(
        timeout_response.contains("{\"error\":\"request timeout\"}"),
        "response: {timeout_response}"
    );

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("must invoke kill");
    assert!(kill_status.success(), "failed to send SIGINT");

    let output = child
        .wait_with_output()
        .expect("must collect rustcode output");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(
        stdout.contains("serve endpoint configured"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"GET\", path: \"/health\", status: 200 }"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"GET\", path: \"/does-not-exist\", status: 404 }"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"\", path: \"\", status: 408 }"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("execution cancelled"), "stdout: {stdout}");
}
