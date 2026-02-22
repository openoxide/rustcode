use super::*;

#[tokio::test]
async fn read_rejects_workspace_escape() {
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
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            workspace_root: PathBuf::from("/tmp/rustcode-workspace"),
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s2".to_string(),
            request_id: "r2".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Read {
                path: "../secret.txt".to_string(),
            },
            context,
            publisher.clone(),
        )
        .await;

    assert!(matches!(result, Err(ExecutionError::Dispatch(_))));

    let events = publisher.events.lock().await.clone();
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Failure { message }
            if message.contains("path escapes workspace root")
        )
    }));
}

#[tokio::test]
async fn plugins_receive_emitted_events() {
    let seen = Arc::new(Mutex::new(0usize));
    let mut registry = PluginRegistry::default();
    registry
        .register(Arc::new(CountingPlugin { seen: seen.clone() }))
        .expect("plugin registration must succeed");

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        registry,
        None,
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());
    let context = CommandContext::new(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "s3".to_string(),
            request_id: "r3".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine.execute(Command::Version, context, publisher).await;
    assert!(result.is_ok());

    let seen_count = *seen.lock().await;
    assert_eq!(seen_count, 3);
}
