use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[test]
fn api_key_round_trip_and_remove() {
    let path = make_temp_file_path("auth-roundtrip");
    let store = AuthStore::with_path(path);

    store
        .set_api_key("openrouter", "secret-123")
        .expect("set key should succeed");
    assert_eq!(
        store
            .get_api_key("openrouter")
            .expect("get key should succeed")
            .as_deref(),
        Some("secret-123")
    );

    let removed = store.remove("openrouter").expect("remove should succeed");
    assert!(removed);
    assert!(store
        .get_api_key("openrouter")
        .expect("get should succeed")
        .is_none());
}

#[test]
fn oauth_round_trip_and_remove() {
    let path = make_temp_file_path("auth-oauth-roundtrip");
    let store = AuthStore::with_path(path);

    store
        .set_oauth(
            "openai",
            "access-token",
            Some("refresh-token"),
            Some(1234),
            Some("account-1"),
        )
        .expect("set oauth should succeed");
    let value = store.get("openai").expect("get oauth should succeed");
    match value {
        Some(StoredCredential::OAuth {
            access_token,
            refresh_token,
            expires_at_unix,
            account_id,
            domain,
        }) => {
            assert_eq!(access_token, "access-token");
            assert_eq!(refresh_token.as_deref(), Some("refresh-token"));
            assert_eq!(expires_at_unix, Some(1234));
            assert_eq!(account_id.as_deref(), Some("account-1"));
            assert!(domain.is_none());
        }
        _ => panic!("expected oauth credential"),
    }
}

#[test]
fn api_key_can_store_domain_metadata() {
    let path = make_temp_file_path("auth-api-key-domain");
    let store = AuthStore::with_path(path);

    store
        .set_api_key_with_domain(
            "github-copilot-enterprise",
            "enterprise-token",
            Some("github.example.com"),
        )
        .expect("set key should succeed");

    let value = store
        .get("github-copilot-enterprise")
        .expect("get should succeed");
    match value {
        Some(StoredCredential::ApiKey { key, domain }) => {
            assert_eq!(key, "enterprise-token");
            assert_eq!(domain.as_deref(), Some("github.example.com"));
        }
        _ => panic!("expected api key credential"),
    }
}

#[test]
fn empty_auth_file_is_treated_as_default_store() {
    let path = make_temp_file_path("auth-empty-file");
    std::fs::write(&path, "").expect("must write empty file");
    let store = AuthStore::with_path(path);
    let providers = store.providers().expect("providers should load");
    assert!(providers.is_empty());
}

#[test]
fn oauth_method_mapping_is_provider_specific() {
    assert_eq!(
        methods_for_provider("openai"),
        vec![
            AuthMethod::OAuthDeviceCode,
            AuthMethod::OAuthBrowser,
            AuthMethod::ApiKey,
        ]
    );
    assert_eq!(
        methods_for_provider("github-copilot"),
        vec![AuthMethod::OAuthDeviceCode, AuthMethod::ApiKey]
    );
    assert_eq!(
        methods_for_provider("gitlab"),
        vec![AuthMethod::OAuthBrowser, AuthMethod::ApiKey]
    );
    assert_eq!(methods_for_provider("openrouter"), vec![AuthMethod::ApiKey]);
}

#[test]
fn oauth_hint_exists_for_supported_provider() {
    let hint = oauth_login_hint("gitlab").expect("hint must exist");
    assert!(hint.authorize_url.contains("gitlab"));
}

#[test]
fn mcp_discovery_paths_include_canonical_and_path_scoped_candidates() {
    assert_eq!(
        mcp_discovery_paths("/"),
        vec!["/.well-known/oauth-authorization-server".to_string()]
    );
    assert_eq!(
        mcp_discovery_paths("/mcp"),
        vec![
            "/.well-known/oauth-authorization-server/mcp".to_string(),
            "/mcp/.well-known/oauth-authorization-server".to_string(),
            "/.well-known/oauth-authorization-server".to_string()
        ]
    );
}

#[tokio::test]
async fn discover_mcp_oauth_rejects_invalid_url() {
    let error = discover_mcp_oauth("not-a-url")
        .await
        .expect_err("invalid url should fail");
    assert!(error.to_string().contains("invalid MCP URL"));
}

#[test]
fn mcp_browser_flow_requires_oauth_endpoints() {
    let discovery = McpOAuthDiscovery {
        supported: true,
        metadata_url: None,
        authorization_endpoint: None,
        token_endpoint: None,
    };
    let error = start_mcp_browser_oauth_flow(
        "github",
        "https://example.com/mcp",
        &discovery,
        "client-123",
        1458,
        &[],
    )
    .expect_err("missing endpoints should fail");
    assert!(error.to_string().contains("authorization_endpoint"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_browser_flow_round_trip_with_mock_token_exchange() {
    let Some(callback_port) = allocate_port() else {
        eprintln!("skipping test: callback port bind is not permitted in this environment");
        return;
    };
    let Some(token_port) = allocate_port() else {
        eprintln!("skipping test: token port bind is not permitted in this environment");
        return;
    };
    let token_endpoint = format!("http://127.0.0.1:{token_port}/oauth/token");
    let authorize_endpoint = format!("http://127.0.0.1:{token_port}/oauth/authorize");
    let discovery = McpOAuthDiscovery {
        supported: true,
        metadata_url: Some(format!(
            "http://127.0.0.1:{token_port}/.well-known/oauth-authorization-server"
        )),
        authorization_endpoint: Some(authorize_endpoint),
        token_endpoint: Some(token_endpoint),
    };
    let capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let capture_clone = capture.clone();
    let Some(server) = spawn_mock_mcp_token_server(token_port, capture_clone) else {
        eprintln!("skipping test: token server bind is not permitted in this environment");
        return;
    };

    let flow = start_mcp_browser_oauth_flow(
        "github",
        "http://127.0.0.1:39443/mcp",
        &discovery,
        "client-123",
        callback_port,
        &["read".to_string(), "write".to_string()],
    )
    .expect("mcp browser flow should start");
    assert_eq!(flow.server_name, "github");
    assert_eq!(
        flow.redirect_uri,
        format!("http://127.0.0.1:{callback_port}/auth/callback")
    );
    let authorize_url =
        reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
    let state = authorize_url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state query parameter should exist");
    let scope = authorize_url
        .query_pairs()
        .find_map(|(key, value)| (key == "scope").then(|| value.into_owned()))
        .expect("scope query parameter should exist");
    assert_eq!(scope, "read write");

    let redirect_uri = flow.redirect_uri.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let callback_url = format!("{redirect_uri}?code=mcp-auth-code&state={state}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("callback client should build");
        let _ = client.get(callback_url).send().await;
    });

    let credential = Box::pin(complete_mcp_browser_oauth_flow(
        &flow,
        Duration::from_secs(5),
        Some("shh"),
    ))
    .await
    .expect("flow should complete");
    assert_eq!(credential.access_token, "mcp-access-token");
    assert_eq!(
        credential.refresh_token.as_deref(),
        Some("mcp-refresh-token")
    );
    assert_eq!(credential.expires_in_secs, Some(3600));

    server.join().expect("mock mcp server should join");
    let body = capture.lock().expect("capture should lock").clone();
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("client_id=client-123"));
    assert!(body.contains("code=mcp-auth-code"));
    assert!(body.contains("code_verifier="));
    assert!(body.contains("client_secret=shh"));
}

#[test]
fn normalizes_domain_for_device_flow() {
    assert_eq!(
        normalize_domain("https://github.com/").expect("must normalize"),
        "github.com"
    );
    assert_eq!(
        normalize_domain("https://gitlab.example.com:8443/root/path").expect("must normalize"),
        "gitlab.example.com:8443"
    );
    assert_eq!(
        normalize_domain("company.ghe.com").expect("must normalize"),
        "company.ghe.com"
    );
}

#[test]
fn parses_interval_from_string_or_number() {
    #[derive(Deserialize)]
    struct Holder {
        #[serde(deserialize_with = "deserialize_u64_string_or_number")]
        value: u64,
    }

    let parsed_number: Holder =
        serde_json::from_str(r#"{"value":5}"#).expect("number interval must parse");
    assert_eq!(parsed_number.value, 5);

    let parsed_string: Holder =
        serde_json::from_str(r#"{"value":"7"}"#).expect("string interval must parse");
    assert_eq!(parsed_string.value, 7);
}

#[tokio::test]
async fn openai_browser_flow_builds_authorize_url() {
    let flow = start_browser_oauth_flow("openai", None, None, 1455)
        .expect("openai browser flow should start");
    assert_eq!(flow.provider, "openai");
    assert_eq!(flow.redirect_uri, "http://127.0.0.1:1455/auth/callback");
    assert!(flow
        .authorize_url
        .contains("https://auth.openai.com/oauth/authorize"));
    assert!(flow
        .authorize_url
        .contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
    assert!(flow.authorize_url.contains("code_challenge_method=S256"));
}

#[tokio::test]
async fn gitlab_browser_flow_uses_bundled_client_id_for_gitlab_com() {
    let flow = start_browser_oauth_flow("gitlab", None, None, 1456)
        .expect("gitlab.com browser flow should start");
    let authorize_url =
        reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
    let client_id = authorize_url
        .query_pairs()
        .find_map(|(key, value)| (key == "client_id").then(|| value.into_owned()))
        .expect("client_id query parameter should exist");
    assert_eq!(client_id, GITLAB_BUNDLED_CLIENT_ID);
}

#[tokio::test]
async fn gitlab_browser_flow_requires_client_id_for_self_hosted() {
    let error = start_browser_oauth_flow("gitlab", Some("gitlab.example.com"), None, 1457)
        .expect_err("self-hosted gitlab flow should require client id");
    assert!(error.to_string().contains("GITLAB_OAUTH_CLIENT_ID"));
}

#[test]
fn extracts_openai_account_id_from_jwt_claims() {
    let payload = serde_json::json!({
        "chatgpt_account_id": "acct_123"
    });
    let encoded =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
    let token = format!("header.{encoded}.signature");
    assert_eq!(
        extract_openai_account_id_from_jwt(&token).as_deref(),
        Some("acct_123")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gitlab_browser_flow_round_trip_with_mock_token_exchange() {
    let Some(callback_port) = allocate_port() else {
        eprintln!("skipping test: callback port bind is not permitted in this environment");
        return;
    };
    let Some(token_port) = allocate_port() else {
        eprintln!("skipping test: token port bind is not permitted in this environment");
        return;
    };
    let token_domain = format!("127.0.0.1:{token_port}");
    let capture = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let capture_clone = capture.clone();
    let Some(server) = spawn_mock_gitlab_token_server(token_port, capture_clone) else {
        eprintln!("skipping test: token server bind is not permitted in this environment");
        return;
    };

    let flow = start_browser_oauth_flow(
        "gitlab",
        Some(&token_domain),
        Some("client-123"),
        callback_port,
    )
    .expect("flow should start");
    let authorize_url =
        reqwest::Url::parse(&flow.authorize_url).expect("authorize url should parse");
    let state = authorize_url
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state query parameter should exist");
    assert!(flow.authorize_url.contains("code_challenge="));

    let redirect_uri = flow.redirect_uri.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let callback_url = format!("{redirect_uri}?code=auth-code-1&state={state}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("callback client should build");
        let _ = client.get(callback_url).send().await;
    });

    let credential = Box::pin(complete_browser_oauth_flow(
        &flow,
        Duration::from_secs(5),
        None,
    ))
    .await
    .expect("flow should complete");
    assert_eq!(credential.access_token, "gitlab-access-token");
    assert_eq!(
        credential.refresh_token.as_deref(),
        Some("gitlab-refresh-token")
    );
    assert_eq!(credential.expires_in_secs, Some(1800));

    server.join().expect("mock gitlab server should join");
    let body = capture.lock().expect("capture should lock").clone();
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("client_id=client-123"));
    assert!(body.contains("code=auth-code-1"));
    assert!(body.contains("code_verifier="));
}

fn make_temp_file_path(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("monotonic time")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("rustcode-auth-{name}-{pid}-{now}.json"))
}

fn allocate_port() -> Option<u16> {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("must bind random port: {err}"),
    };
    let port = listener.local_addr().expect("must read local addr").port();
    drop(listener);
    Some(port)
}

fn spawn_mock_gitlab_token_server(
    port: u16,
    body_capture: std::sync::Arc<std::sync::Mutex<String>>,
) -> Option<std::thread::JoinHandle<()>> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("must bind token port: {err}"),
    };
    Some(thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("must configure listener nonblocking");
        // If the token exchange never happens (e.g., upstream request fails), we must not hang
        // the entire test suite waiting on accept/join.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut accepted: Option<TcpStream> = None;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    accepted = Some(stream);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("must accept token request: {err}"),
            }
        }
        let Some(mut stream) = accepted else {
            // Let the assertions in the test fail on empty capture instead of hanging forever.
            return;
        };
        let request = read_http_request(&mut stream);
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        *body_capture
            .lock()
            .expect("body capture must lock for write") = body;
        let response_body = r#"{"access_token":"gitlab-access-token","refresh_token":"gitlab-refresh-token","expires_in":1800}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("must write token response");
    }))
}

fn spawn_mock_mcp_token_server(
    port: u16,
    body_capture: std::sync::Arc<std::sync::Mutex<String>>,
) -> Option<std::thread::JoinHandle<()>> {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("must bind token port: {err}"),
    };
    Some(thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("must configure listener nonblocking");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut accepted: Option<TcpStream> = None;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => {
                    accepted = Some(stream);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("must accept token request: {err}"),
            }
        }
        let Some(mut stream) = accepted else {
            return;
        };
        let request = read_http_request(&mut stream);
        let body = request
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        *body_capture
            .lock()
            .expect("body capture must lock for write") = body;
        let response_body = r#"{"access_token":"mcp-access-token","refresh_token":"mcp-refresh-token","expires_in":3600}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("must write token response");
    }))
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = [0_u8; 16 * 1024];
    let mut request = Vec::new();
    let mut content_length = 0_usize;
    let mut header_end_index: Option<usize> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                request.extend_from_slice(&buffer[..read]);
                if header_end_index.is_none() {
                    if let Some(index) =
                        request.windows(4).position(|chunk| chunk == b"\r\n\r\n")
                    {
                        let end = index + 4;
                        header_end_index = Some(end);
                        let headers = String::from_utf8_lossy(&request[..end]);
                        for line in headers.lines() {
                            let lower = line.to_ascii_lowercase();
                            if let Some(value) = lower.strip_prefix("content-length:") {
                                content_length = value.trim().parse::<usize>().unwrap_or(0);
                                break;
                            }
                        }
                    }
                }
                if let Some(end) = header_end_index {
                    if request.len() >= end + content_length {
                        break;
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("must read request: {err}"),
        }
    }
    String::from_utf8(request).expect("request must be utf8")
}
