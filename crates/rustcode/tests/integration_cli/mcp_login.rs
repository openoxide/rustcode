use super::*;

#[test]
fn mcp_login_list_logout_round_trip() {
    let auth_path = make_temp_file_path("mcp-auth-roundtrip");

    let login_output = Command::new(rustcode_bin())
        .args(["mcp", "login", "github", "--from-env", "RUSTCODE_MCP_TOKEN"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_MCP_TOKEN", "mcp-secret")
        .output()
        .expect("must run rustcode mcp login");
    assert!(
        login_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&login_output.stdout),
        String::from_utf8_lossy(&login_output.stderr)
    );

    let list_output = Command::new(rustcode_bin())
        .args(["mcp", "list"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode mcp list");
    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let list_stdout = String::from_utf8(list_output.stdout).expect("stdout must be utf8");
    assert!(list_stdout.contains("servers=1"));
    assert!(list_stdout.contains("name=github\tcredential=stored:api_key"));

    let logout_output = Command::new(rustcode_bin())
        .args(["mcp", "logout", "github"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode mcp logout");
    assert!(
        logout_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&logout_output.stdout),
        String::from_utf8_lossy(&logout_output.stderr)
    );
    let logout_stdout = String::from_utf8(logout_output.stdout).expect("stdout must be utf8");
    assert!(logout_stdout.contains("removed mcp credential for name=github"));
}

#[test]
fn mcp_json_contracts_are_parseable() {
    let auth_path = make_temp_file_path("mcp-auth-json");

    let login_output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--from-env",
            "RUSTCODE_MCP_TOKEN",
            "--scopes",
            "read,write",
            "--url",
            "https://example.com/mcp",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_MCP_TOKEN", "mcp-secret")
        .output()
        .expect("must run rustcode mcp login");
    assert!(
        login_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&login_output.stdout),
        String::from_utf8_lossy(&login_output.stderr)
    );
    let login_payload: Value = serde_json::from_str(
        String::from_utf8(login_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(login_payload["schema_version"].as_u64(), Some(1));
    assert_eq!(login_payload["command"].as_str(), Some("mcp.login"));
    assert_eq!(login_payload["name"].as_str(), Some("github"));
    assert_eq!(login_payload["stage"].as_str(), Some("authorized"));

    let list_output = Command::new(rustcode_bin())
        .args(["--json", "mcp", "list"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode mcp list");
    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let list_payload: Value = serde_json::from_str(
        String::from_utf8(list_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(list_payload["command"].as_str(), Some("mcp.list"));
    assert!(list_payload["servers"]
        .as_array()
        .expect("servers should be array")
        .iter()
        .any(|row| row["name"].as_str() == Some("github")));

    let logout_output = Command::new(rustcode_bin())
        .args(["--json", "mcp", "logout", "github"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode mcp logout");
    assert!(
        logout_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&logout_output.stdout),
        String::from_utf8_lossy(&logout_output.stderr)
    );
    let logout_payload: Value = serde_json::from_str(
        String::from_utf8(logout_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(logout_payload["command"].as_str(), Some("mcp.logout"));
    assert_eq!(logout_payload["removed"].as_bool(), Some(true));
}

#[test]
fn mcp_login_resolves_url_from_configured_server() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let expected_url = format!("http://127.0.0.1:{port}/mcp");
    let auth_path = make_temp_file_path("mcp-auth-config-login");
    let config_path = make_temp_file_path("mcp-servers-config-login");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "github": {{ "url": "{expected_url}", "oauth": true }}
}}"#
        ),
    )
    .expect("must write mcp server config fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "mcp", "login", "github", "--scopes", "read,write"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_MCP_SERVERS_PATH", &config_path)
        .output()
        .expect("must run rustcode mcp login");

    handle.join().expect("discovery server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");
    let discovered: Value = serde_json::from_str(lines[0]).expect("discovered line must parse");
    assert_eq!(discovered["name"].as_str(), Some("github"));
    assert_eq!(discovered["stage"].as_str(), Some("oauth_discovered"));
    assert_eq!(discovered["url"].as_str(), Some(expected_url.as_str()));
}

#[test]
fn mcp_login_requires_from_env_in_non_interactive_mode() {
    let output = Command::new(rustcode_bin())
        .args(["mcp", "login", "github"])
        .output()
        .expect("must run rustcode mcp login");

    assert!(!output.status.success(), "command should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr must be utf8");
    assert!(stderr.contains("requires either --from-env"));
}

#[test]
fn mcp_login_missing_mode_json_emits_failed_envelope() {
    let output = Command::new(rustcode_bin())
        .args(["--json", "mcp", "login", "github"])
        .output()
        .expect("must run rustcode mcp login");

    assert!(!output.status.success(), "command should fail");
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(payload["schema_version"].as_u64(), Some(1));
    assert_eq!(payload["command"].as_str(), Some("mcp.login"));
    assert_eq!(payload["name"].as_str(), Some("github"));
    assert_eq!(payload["stage"].as_str(), Some("failed"));
    assert_eq!(payload["error_kind"].as_str(), Some("validation"));
}

#[test]
fn mcp_login_rejects_from_env_with_oauth_browser_method() {
    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--from-env",
            "RUSTCODE_MCP_TOKEN",
            "--method",
            "oauth_browser",
        ])
        .env("RUSTCODE_MCP_TOKEN", "mcp-secret")
        .output()
        .expect("must run rustcode mcp login");

    assert!(!output.status.success(), "command should fail");
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let payload: Value = serde_json::from_str(stdout.trim()).expect("stdout should be json");
    assert_eq!(payload["stage"].as_str(), Some("failed"));
    assert_eq!(payload["error_kind"].as_str(), Some("validation"));
    assert!(payload["error"]
        .as_str()
        .expect("error should be set")
        .contains("cannot be combined"));
}

#[test]
fn mcp_login_oauth_discovery_json_emits_staged_contract() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let url = format!("http://127.0.0.1:{port}/mcp");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--url",
            &url,
            "--scopes",
            "read,write",
        ])
        .output()
        .expect("must run rustcode mcp login");

    handle.join().expect("discovery server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");

    let discovered: Value = serde_json::from_str(lines[0]).expect("discovered line must parse");
    assert_eq!(discovered["schema_version"].as_u64(), Some(1));
    assert_eq!(discovered["command"].as_str(), Some("mcp.login"));
    assert_eq!(discovered["name"].as_str(), Some("github"));
    assert_eq!(discovered["stage"].as_str(), Some("oauth_discovered"));
    assert_eq!(
        discovered["authorization_endpoint"].as_str(),
        Some("https://mcp.example.com/authorize")
    );
    assert_eq!(
        discovered["token_endpoint"].as_str(),
        Some("https://mcp.example.com/token")
    );
    assert_eq!(
        discovered["scopes"]
            .as_array()
            .expect("scopes must be array")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec!["read", "write"]
    );

    let awaiting: Value = serde_json::from_str(lines[1]).expect("awaiting line must parse");
    assert_eq!(awaiting["schema_version"].as_u64(), Some(1));
    assert_eq!(awaiting["command"].as_str(), Some("mcp.login"));
    assert_eq!(awaiting["name"].as_str(), Some("github"));
    assert_eq!(awaiting["stage"].as_str(), Some("awaiting_token_import"));
}

#[test]
fn mcp_login_oauth_browser_no_wait_json_emits_stage_sequence() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let url = format!("http://127.0.0.1:{port}/mcp");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--method",
            "oauth_browser",
            "--url",
            &url,
            "--client-id",
            "client-123",
            "--no-wait",
            "--oauth-port",
            "19440",
            "--scopes",
            "read,write",
        ])
        .output()
        .expect("must run rustcode mcp login");

    handle.join().expect("discovery server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");

    let discovered: Value = serde_json::from_str(lines[0]).expect("discovered line must parse");
    assert_eq!(discovered["method"].as_str(), Some("oauth_browser"));
    assert_eq!(discovered["stage"].as_str(), Some("oauth_discovered"));

    let challenge: Value = serde_json::from_str(lines[1]).expect("challenge line must parse");
    assert_eq!(challenge["method"].as_str(), Some("oauth_browser"));
    assert_eq!(challenge["stage"].as_str(), Some("challenge"));
    assert_eq!(challenge["oauth_port"].as_u64(), Some(19440));
    assert!(challenge["authorize_url"]
        .as_str()
        .expect("authorize_url should be set")
        .contains("client_id=client-123"));
    assert_eq!(
        challenge["redirect_uri"].as_str(),
        Some("http://127.0.0.1:19440/auth/callback")
    );

    let awaiting: Value = serde_json::from_str(lines[2]).expect("awaiting line must parse");
    assert_eq!(awaiting["method"].as_str(), Some("oauth_browser"));
    assert_eq!(
        awaiting["stage"].as_str(),
        Some("awaiting_browser_callback")
    );
}

#[test]
fn mcp_login_oauth_browser_uses_configured_client_id() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let auth_path = make_temp_file_path("mcp-auth-config-browser");
    let config_path = make_temp_file_path("mcp-servers-config-browser");
    std::fs::write(
        &config_path,
        format!(
            r#"{{
  "github": {{
    "url": "http://127.0.0.1:{port}/mcp",
    "oauth": {{ "enabled": true, "client_id": "configured-client-id" }}
  }}
}}"#
        ),
    )
    .expect("must write mcp server config fixture");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--method",
            "oauth_browser",
            "--no-wait",
            "--oauth-port",
            "19441",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_MCP_SERVERS_PATH", &config_path)
        .output()
        .expect("must run rustcode mcp login");

    handle.join().expect("discovery server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");
    let challenge: Value = serde_json::from_str(lines[1]).expect("challenge line must parse");
    assert!(challenge["authorize_url"]
        .as_str()
        .expect("authorize_url should be set")
        .contains("client_id=configured-client-id"));
}

#[test]
fn mcp_login_oauth_browser_requires_client_id_json_envelope() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let url = format!("http://127.0.0.1:{port}/mcp");

    let output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "login",
            "github",
            "--method",
            "oauth_browser",
            "--url",
            &url,
            "--no-wait",
        ])
        .output()
        .expect("must run rustcode mcp login");

    handle.join().expect("discovery server should join");

    assert!(!output.status.success(), "command should fail");
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "stdout: {stdout}");

    let discovered: Value = serde_json::from_str(lines[0]).expect("discovered line must parse");
    assert_eq!(discovered["stage"].as_str(), Some("oauth_discovered"));
    let failure: Value = serde_json::from_str(lines[1]).expect("failure line must parse");
    assert_eq!(failure["stage"].as_str(), Some("failed"));
    assert_eq!(failure["error_kind"].as_str(), Some("validation"));
    assert!(failure["error"]
        .as_str()
        .expect("error should be set")
        .contains("client-id"));
}
