use super::fixtures::*;
use super::*;

#[tokio::test]
async fn webfetch_rejects_when_network_disabled() {
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            allow_network: false,
            permission_rules: vec![rustcode_core::PermissionRule {
                permission: "webfetch".to_string(),
                action: PermissionAction::Allow,
                pattern: "*".to_string(),
            }],
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-webfetch-1".to_string(),
            request_id: "r-webfetch-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "webfetch",
            r#"{"url":"http://127.0.0.1/"}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    match result {
        Err(ExecutionError::Dispatch(message)) => {
            assert!(
                message.contains("network access is disabled"),
                "message={message}"
            );
        }
        other => panic!("expected dispatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn webfetch_fetches_local_http_and_simplifies_html_by_default() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener must bind");
    let addr = listener.local_addr().expect("listener addr");

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 1024];
        let _ = timeout(Duration::from_secs(1), socket.read(&mut buf)).await;

        let body = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            allow_network: true,
            permission_rules: vec![rustcode_core::PermissionRule {
                permission: "webfetch".to_string(),
                action: PermissionAction::Allow,
                pattern: "*".to_string(),
            }],
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-webfetch-2".to_string(),
            request_id: "r-webfetch-2".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let url = format!("http://{addr}/");
    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "webfetch",
            &format!(r#"{{"url":"{url}"}}"#),
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("webfetch should succeed");

    server.await.expect("server must join");

    assert!(output.contains("status=200"), "output={output}");
    assert!(output.contains("Hello"), "output={output}");
    assert!(output.contains("World"), "output={output}");
    assert!(!output.contains("<html>"), "output={output}");
}

#[tokio::test]
async fn webfetch_format_html_returns_raw_html() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener must bind");
    let addr = listener.local_addr().expect("listener addr");

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 1024];
        let _ = timeout(Duration::from_secs(1), socket.read(&mut buf)).await;

        let body = "<html><body>ok</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            allow_network: true,
            permission_rules: vec![rustcode_core::PermissionRule {
                permission: "webfetch".to_string(),
                action: PermissionAction::Allow,
                pattern: "*".to_string(),
            }],
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-webfetch-3".to_string(),
            request_id: "r-webfetch-3".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let url = format!("http://{addr}/");
    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "webfetch",
            &format!(r#"{{"url":"{url}","format":"html"}}"#),
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("webfetch should succeed");

    server.await.expect("server must join");

    assert!(output.contains("<html>"), "output={output}");
}
