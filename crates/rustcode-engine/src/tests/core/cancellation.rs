use super::*;

#[tokio::test]
async fn cancellation_emits_warning_and_returns_cancelled() {
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
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "s1".to_string(),
            request_id: "r1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Exec {
                command: "sleep".to_string(),
                args: vec!["30".to_string()],
            },
            context,
            publisher.clone(),
        )
        .await;

    assert!(matches!(result, Err(ExecutionError::Cancelled)));

    let events = publisher.events.lock().await.clone();
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::CommandAccepted { name }
            if name.contains("Exec")
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Warning { message }
            if message == "execution cancelled"
        )
    }));
    assert!(!events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::Completed)));
}
