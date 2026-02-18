use super::*;

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
