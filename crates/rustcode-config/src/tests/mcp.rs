use super::*;

#[test]
fn mcp_server_config_layers_and_merges() {
    let temp_root = make_temp_dir("mcp-config");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");

    let global = temp_root.join("global.toml");
    let user = temp_root.join("user.toml");
    let project = cwd.join("rustcode.toml");

    write_config(
        &global,
        r#"
[mcp.servers.github]
url = "https://global.example.com/mcp"
oauth = true
"#,
    );
    write_config(
        &user,
        &format!(
            r#"
[mcp.servers.github]
url = "https://user.example.com/mcp"
oauth = {{ enabled = true, client_id = "user-client" }}

[trust]
projects = ["{}"]
"#,
            cwd.display()
        ),
    );
    write_config(
        &project,
        r#"
[mcp.servers.github]
oauth = { enabled = false }

[mcp.servers.linear]
url = "https://linear.example.com/mcp"
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);
    sources.user_config_path = Some(user);
    sources.project_config_path = Some(project);
    sources.trust_project = true;

    let cfg = ConfigLoader::load(&sources).expect("config should load");
    assert_eq!(cfg.mcp_servers.len(), 2);

    let github = cfg
        .mcp_servers
        .get("github")
        .expect("github mcp config should exist");
    assert_eq!(github.url.as_deref(), Some("https://user.example.com/mcp"));
    assert!(!github.oauth.enabled);
    assert_eq!(github.oauth.client_id.as_deref(), Some("user-client"));

    let linear = cfg
        .mcp_servers
        .get("linear")
        .expect("linear mcp config should exist");
    assert_eq!(
        linear.url.as_deref(),
        Some("https://linear.example.com/mcp")
    );
    assert!(linear.oauth.enabled);
}

#[test]
fn mcp_server_config_rejects_empty_url() {
    let temp_root = make_temp_dir("mcp-config-invalid");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");
    let global = temp_root.join("global.toml");

    write_config(
        &global,
        r#"
[mcp.servers.github]
url = "   "
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);

    let err = ConfigLoader::load(&sources).expect_err("must reject empty mcp url");
    match err {
        ConfigError::Validation(message) => {
            assert!(message.contains("mcp.servers.github.url"));
        }
        _ => panic!("expected validation error"),
    }
}

#[test]
fn mcp_server_config_accepts_stdio_transport_fields() {
    let temp_root = make_temp_dir("mcp-config-stdio");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");
    let global = temp_root.join("global.toml");

    write_config(
        &global,
        r#"
[mcp.servers.local]
command = "node"
args = ["server.js", "--mcp"]
env = { MCP_MODE = "test" }
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);

    let cfg = ConfigLoader::load(&sources).expect("must accept stdio mcp config");
    let local = cfg
        .mcp_servers
        .get("local")
        .expect("local mcp config should exist");
    assert!(local.url.is_none());
    assert_eq!(local.command.as_deref(), Some("node"));
    assert_eq!(local.args, vec!["server.js", "--mcp"]);
    assert_eq!(local.env.get("MCP_MODE").map(String::as_str), Some("test"));
}

#[test]
fn mcp_server_config_rejects_missing_transport() {
    let temp_root = make_temp_dir("mcp-config-no-transport");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");
    let global = temp_root.join("global.toml");

    write_config(
        &global,
        r#"
[mcp.servers.invalid]
oauth = true
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);

    let err = ConfigLoader::load(&sources).expect_err("must reject missing transport");
    match err {
        ConfigError::Validation(message) => {
            assert!(message.contains("must set either url or command"));
        }
        _ => panic!("expected validation error"),
    }
}

#[test]
fn mcp_server_config_rejects_both_http_and_stdio_transport() {
    let temp_root = make_temp_dir("mcp-config-both-transport");
    let cwd = temp_root.join("project");
    fs::create_dir_all(&cwd).expect("must create cwd");
    let global = temp_root.join("global.toml");

    write_config(
        &global,
        r#"
[mcp.servers.invalid]
url = "https://example.com/mcp"
command = "node"
"#,
    );

    let mut sources = ConfigSources::new(cwd);
    sources.read_process_env = false;
    sources.global_config_path = Some(global);

    let err = ConfigLoader::load(&sources).expect_err("must reject dual transport");
    match err {
        ConfigError::Validation(message) => {
            assert!(message.contains("cannot set both url and command"));
        }
        _ => panic!("expected validation error"),
    }
}
