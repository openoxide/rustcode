use super::fixtures::*;
use super::*;
use crate::mcp::McpRegistry;

#[tokio::test]
async fn mcp_tool_call_executes_via_registry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let server = tokio::spawn(async move {
        for step in 0..3 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            if step == 0 {
                assert!(
                    request.contains("\"method\":\"initialize\""),
                    "request={request}"
                );
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "result": {
                        "protocolVersion":"2025-11-25",
                        "capabilities":{},
                        "serverInfo":{"name":"stub","version":"0"}
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: sess-mcp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            } else if step == 1 {
                assert!(
                    request.contains("notifications/initialized"),
                    "request={request}"
                );
                let response =
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                socket.write_all(response.as_bytes()).await.expect("write");
            } else {
                assert!(
                    request.contains("\"method\":\"tools/call\""),
                    "request={request}"
                );
                assert!(request.contains("\"name\":\"hello\""), "request={request}");
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "result": {
                        "content": [{"type":"text","text":"ok"}],
                        "isError": false
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            }
        }
    });

    let mut registry = McpRegistry::new();
    registry
        .connect("demo".to_string(), &format!("http://{addr}/mcp"), None)
        .await
        .expect("connect mcp registry");

    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "mcp".to_string(),
            action: PermissionAction::Allow,
            pattern: "mcp:demo:hello".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-mcp-call-1".to_string(),
            request_id: "r-mcp-call-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    )
    .with_mcp(registry);

    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "mcp:demo:hello",
            r"{}",
            &context,
            &AgentOptions::default(),
            &mut state,
        )
        .await
        .expect("mcp call should succeed");
    assert!(output.contains("\"isError\":false"), "output={output}");

    server.await.expect("server join");
}

#[tokio::test]
async fn mcp_resource_read_executes_via_registry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let server = tokio::spawn(async move {
        for step in 0..3 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            if step == 0 {
                assert!(
                    request.contains("\"method\":\"initialize\""),
                    "request={request}"
                );
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "result": {
                        "protocolVersion":"2025-11-25",
                        "capabilities":{},
                        "serverInfo":{"name":"stub","version":"0"}
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMCP-Session-Id: sess-mcp-res\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            } else if step == 1 {
                assert!(
                    request.contains("notifications/initialized"),
                    "request={request}"
                );
                let response =
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                socket.write_all(response.as_bytes()).await.expect("write");
            } else {
                assert!(
                    request.contains("\"method\":\"resources/read\""),
                    "request={request}"
                );
                assert!(
                    request.contains("\"uri\":\"file:///tmp/demo\""),
                    "request={request}"
                );
                let body = serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":2,
                    "result": {
                        "contents": [
                            {"uri":"file:///tmp/demo","mimeType":"text/plain","text":"hello"}
                        ]
                    }
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            }
        }
    });

    let mut registry = McpRegistry::new();
    registry
        .connect("demo".to_string(), &format!("http://{addr}/mcp"), None)
        .await
        .expect("connect mcp registry");

    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "mcp".to_string(),
            action: PermissionAction::Allow,
            pattern: "mcp:demo:__resources_read".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-mcp-resource-read-1".to_string(),
            request_id: "r-mcp-resource-read-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    )
    .with_mcp(registry);

    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "mcp:demo:__resources_read",
            r#"{"uri":"file:///tmp/demo"}"#,
            &context,
            &AgentOptions::default(),
            &mut state,
        )
        .await
        .expect("mcp resource read should succeed");
    assert!(output.contains("\"contents\""), "output={output}");
    assert!(output.contains("\"hello\""), "output={output}");

    server.await.expect("server join");
}
