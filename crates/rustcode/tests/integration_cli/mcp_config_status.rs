use super::*;

#[test]
fn mcp_list_includes_configured_servers_without_credentials() {
    let auth_path = make_temp_file_path("mcp-auth-config-list");
    let config_path = make_temp_file_path("mcp-servers-config-list");
    std::fs::write(
        &config_path,
        r#"{
  "github": { "url": "https://mcp.github.local/sse", "oauth": true },
  "readonly": { "url": "https://mcp.readonly.local/sse", "oauth": false }
}"#,
    )
    .expect("must write mcp server config fixture");

    let output = Command::new(rustcode_bin())
        .args(["--json", "mcp", "list"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_MCP_SERVERS_PATH", &config_path)
        .output()
        .expect("must run rustcode mcp list");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_str(
        String::from_utf8(output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    let servers = payload["servers"]
        .as_array()
        .expect("servers should be array");
    let github = servers
        .iter()
        .find(|row| row["name"].as_str() == Some("github"))
        .expect("github row should exist");
    assert_eq!(github["credential"].as_str(), Some("none"));
    assert_eq!(github["configured"].as_bool(), Some(true));
    assert_eq!(github["url"].as_str(), Some("https://mcp.github.local/sse"));
    assert_eq!(github["oauth_enabled"].as_bool(), Some(true));

    let readonly = servers
        .iter()
        .find(|row| row["name"].as_str() == Some("readonly"))
        .expect("readonly row should exist");
    assert_eq!(readonly["credential"].as_str(), Some("none"));
    assert_eq!(readonly["configured"].as_bool(), Some(true));
    assert_eq!(readonly["oauth_enabled"].as_bool(), Some(false));
}

#[test]
fn mcp_list_reads_project_config_mcp_servers_when_trusted() {
    let root = std::env::temp_dir().join(format!(
        "rustcode-mcp-project-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("must create project root");
    std::fs::write(
        root.join("rustcode.toml"),
        r#"
[mcp.servers.github]
url = "https://accounts.google.com"
oauth = { enabled = true, client_id = "cfg-client" }
"#,
    )
    .expect("must write project config");

    let output = Command::new(rustcode_bin())
        .args(["--trust-project-config", "--json", "mcp", "list"])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp list");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_str(
        String::from_utf8(output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    let servers = payload["servers"]
        .as_array()
        .expect("servers should be array");
    assert!(servers
        .iter()
        .any(|row| row["name"].as_str() == Some("github")));
}

#[test]
fn mcp_add_writes_project_config_and_list_sees_server_when_trusted() {
    let root = std::env::temp_dir().join(format!(
        "rustcode-mcp-add-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("must create project root");

    let add_output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "add",
            "github",
            "--url",
            "https://accounts.google.com",
            "--oauth",
            "on",
            "--client-id",
            "cfg-client",
            "--scope",
            "project",
        ])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp add");

    assert!(
        add_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add_output.stdout),
        String::from_utf8_lossy(&add_output.stderr)
    );
    let add_payload: Value = serde_json::from_str(
        String::from_utf8(add_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(add_payload["command"].as_str(), Some("mcp.add"));
    assert_eq!(add_payload["scope"].as_str(), Some("project"));

    let list_output = Command::new(rustcode_bin())
        .args(["--trust-project-config", "--json", "mcp", "list"])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp list");

    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let payload: Value = serde_json::from_str(
        String::from_utf8(list_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    let servers = payload["servers"]
        .as_array()
        .expect("servers should be array");
    let row = servers
        .iter()
        .find(|row| row["name"].as_str() == Some("github"))
        .expect("github row should exist");
    assert_eq!(row["configured"].as_bool(), Some(true));
    assert_eq!(row["url"].as_str(), Some("https://accounts.google.com"));
}

#[test]
fn mcp_add_stdio_writes_project_config_and_list_reports_stdio_transport() {
    let root = std::env::temp_dir().join(format!(
        "rustcode-mcp-add-stdio-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("must create project root");

    let add_output = Command::new(rustcode_bin())
        .args([
            "--json",
            "mcp",
            "add",
            "local",
            "--command",
            "node",
            "--arg",
            "server.js",
            "--arg=--mcp",
            "--env",
            "MCP_MODE=test",
            "--scope",
            "project",
        ])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp add");

    assert!(
        add_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add_output.stdout),
        String::from_utf8_lossy(&add_output.stderr)
    );
    let add_payload: Value = serde_json::from_str(
        String::from_utf8(add_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(add_payload["command"].as_str(), Some("mcp.add"));
    assert_eq!(add_payload["transport"].as_str(), Some("stdio"));
    assert_eq!(add_payload["command_name"].as_str(), Some("node"));

    let list_output = Command::new(rustcode_bin())
        .args(["--trust-project-config", "--json", "mcp", "list"])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp list");

    assert!(
        list_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&list_output.stdout),
        String::from_utf8_lossy(&list_output.stderr)
    );
    let payload: Value = serde_json::from_str(
        String::from_utf8(list_output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    let servers = payload["servers"]
        .as_array()
        .expect("servers should be array");
    let row = servers
        .iter()
        .find(|row| row["name"].as_str() == Some("local"))
        .expect("local row should exist");
    assert_eq!(row["configured"].as_bool(), Some(true));
    assert_eq!(row["transport"].as_str(), Some("stdio"));
    assert_eq!(row["command"].as_str(), Some("node"));
    assert_eq!(
        row["args"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("server.js")
    );
}

#[test]
fn mcp_status_reports_oauth_supported_for_discoverable_server() {
    let Some((port, handle)) = spawn_mcp_discovery_server() else {
        return;
    };
    let root = std::env::temp_dir().join(format!(
        "rustcode-mcp-status-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("must create project root");
    std::fs::write(
        root.join("rustcode.toml"),
        format!(
            r#"
[mcp.servers.github]
url = "http://127.0.0.1:{port}/mcp"
oauth = true
"#
        ),
    )
    .expect("must write project config");

    let output = Command::new(rustcode_bin())
        .args(["--trust-project-config", "--json", "mcp", "status"])
        .current_dir(&root)
        .output()
        .expect("must run rustcode mcp status");

    handle.join().expect("discovery server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_str(
        String::from_utf8(output.stdout)
            .expect("stdout must be utf8")
            .trim(),
    )
    .expect("must parse json");
    assert_eq!(payload["command"].as_str(), Some("mcp.status"));
    let servers = payload["servers"]
        .as_array()
        .expect("servers should be array");
    let row = servers
        .iter()
        .find(|row| row["name"].as_str() == Some("github"))
        .expect("github row should exist");
    assert_eq!(row["oauth_enabled"].as_bool(), Some(true));
    assert_eq!(row["oauth_supported"].as_bool(), Some(true));
    assert_eq!(
        row["discovery"]["authorization_endpoint"].as_str(),
        Some("https://mcp.example.com/authorize")
    );
    assert_eq!(
        row["discovery"]["token_endpoint"].as_str(),
        Some("https://mcp.example.com/token")
    );
}
