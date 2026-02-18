use super::fixtures::*;
use super::*;

#[tokio::test]
async fn serve_v1_run_streams_events_over_sse() {
    let _guard = ENV_LOCK.lock().expect("lock");
    let sessions_root = std::env::temp_dir().join(format!(
        "rustcode-engine-serve-sessions-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&sessions_root);
    std::fs::create_dir_all(&sessions_root).expect("create sessions root");
    let prev_sessions_dir = std::env::var("RUSTCODE_SESSIONS_DIR").ok();
    std::env::set_var("RUSTCODE_SESSIONS_DIR", &sessions_root);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let cancellation = CancellationToken::new();
    let context = CommandContext::with_cancellation(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "serve-run-s1".to_string(),
            request_id: "serve-run-r1".to_string(),
            started_at: SystemTime::now(),
        },
        cancellation.clone(),
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());

    let task = tokio::spawn({
        let publisher = publisher.clone();
        async move {
            engine
                .execute(
                    Command::Serve {
                        listen: format!("127.0.0.1:{port}"),
                    },
                    context,
                    publisher,
                )
                .await
        }
    });

    for _ in 0..50 {
        let events = publisher.events.lock().await.clone();
        let configured = events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::Warning { message }
                if message.contains("serve endpoint configured")
            )
        });
        if configured {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let body = serde_json::json!({
        "schema_version": 1,
        "prompt": "hello",
    })
    .to_string();
    let request = format!(
        "POST /v1/run HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(request.as_bytes()).await.expect("write");

    let mut response = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("read must complete")
        .expect("read");
    let response = String::from_utf8_lossy(&response).to_string();
    assert!(response.contains("200 OK"), "response={response}");
    assert!(
        response.to_ascii_lowercase().contains("text/event-stream"),
        "response={response}"
    );
    assert!(response.contains("data:"), "response={response}");
    assert!(response.contains("null-llm response"), "response={response}");

    cancellation.cancel();
    let _ = task.await.expect("server task join");

    if let Some(prev) = prev_sessions_dir {
        std::env::set_var("RUSTCODE_SESSIONS_DIR", prev);
    } else {
        std::env::remove_var("RUSTCODE_SESSIONS_DIR");
    }
}

#[tokio::test]
async fn serve_sessions_endpoints_create_list_show_and_persist_run() {
    let _guard = ENV_LOCK.lock().expect("lock");
    let sessions_root = std::env::temp_dir().join(format!(
        "rustcode-engine-serve-sessions-api-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&sessions_root);
    std::fs::create_dir_all(&sessions_root).expect("create sessions root");
    let prev_sessions_dir = std::env::var("RUSTCODE_SESSIONS_DIR").ok();
    std::env::set_var("RUSTCODE_SESSIONS_DIR", &sessions_root);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let cancellation = CancellationToken::new();
    let context = CommandContext::with_cancellation(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "serve-sessions-s1".to_string(),
            request_id: "serve-sessions-r1".to_string(),
            started_at: SystemTime::now(),
        },
        cancellation.clone(),
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());

    let task = tokio::spawn({
        let publisher = publisher.clone();
        async move {
            engine
                .execute(
                    Command::Serve {
                        listen: format!("127.0.0.1:{port}"),
                    },
                    context,
                    publisher,
                )
                .await
        }
    });

    for _ in 0..50 {
        let events = publisher.events.lock().await.clone();
        let configured = events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::Warning { message }
                if message.contains("serve endpoint configured")
            )
        });
        if configured {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let body = serde_json::json!({"schema_version": 1, "title": "t1"}).to_string();
    let request = format!(
        "POST /v1/sessions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("read")
        .expect("read");
    let response = String::from_utf8_lossy(&response).to_string();
    assert!(response.contains("201 Created"), "response={response}");
    let json_start = response.find("\r\n\r\n").expect("header end") + 4;
    let payload: serde_json::Value =
        serde_json::from_str(response[json_start..].trim()).expect("json");
    let session_id = payload["session"]["id"].as_str().expect("id").to_string();

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let request = "GET /v1/sessions HTTP/1.1\r\nHost: localhost\r\n\r\n";
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("read")
        .expect("read");
    let response = String::from_utf8_lossy(&response).to_string();
    assert!(response.contains("200 OK"), "response={response}");
    assert!(response.contains(&session_id), "response={response}");

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let body = serde_json::json!({
        "schema_version": 1,
        "prompt": "hello",
        "session_id": session_id.clone(),
    })
    .to_string();
    let request = format!(
        "POST /v1/run HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("read")
        .expect("read");
    let response = String::from_utf8_lossy(&response).to_string();
    assert!(
        response.contains("text/event-stream"),
        "response={response}"
    );

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let request = format!("GET /v1/sessions/{session_id} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    socket.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("read")
        .expect("read");
    let response = String::from_utf8_lossy(&response).to_string();
    let json_start = response.find("\r\n\r\n").expect("header end") + 4;
    let payload: serde_json::Value =
        serde_json::from_str(response[json_start..].trim()).expect("json");
    let message_count = payload["messages"]
        .as_array()
        .map(|items| items.len())
        .unwrap_or(0);
    assert!(message_count >= 2, "payload={payload}");

    cancellation.cancel();
    let _ = task.await.expect("server task join");

    if let Some(prev) = prev_sessions_dir {
        std::env::set_var("RUSTCODE_SESSIONS_DIR", prev);
    } else {
        std::env::remove_var("RUSTCODE_SESSIONS_DIR");
    }
}
