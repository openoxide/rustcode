use super::*;

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
