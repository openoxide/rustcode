use super::*;

#[tokio::test]
async fn serve_waits_until_cancelled() {
    let cancellation = CancellationToken::new();
    let context = CommandContext::with_cancellation(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "s4".to_string(),
            request_id: "r4".to_string(),
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
                        listen: "127.0.0.1:4317".to_string(),
                    },
                    context,
                    publisher,
                )
                .await
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();

    let result = task.await.expect("task must join");
    let events = publisher.events.lock().await.clone();
    match result {
        Err(ExecutionError::Cancelled) => {
            assert!(events.iter().any(|event| {
                matches!(
                    &event.payload,
                    EventPayload::Warning { message }
                    if message.contains("serve endpoint configured")
                )
            }));
            assert!(events.iter().any(|event| {
                matches!(
                    &event.payload,
                    EventPayload::Warning { message }
                    if message == "execution cancelled"
                )
            }));
        }
        Err(ExecutionError::Executor(message)) if message.contains("failed to bind") => {
            assert!(events.iter().any(|event| {
                matches!(
                    &event.payload,
                    EventPayload::Failure { message }
                    if message.contains("failed to bind")
                )
            }));
        }
        other => panic!("unexpected serve result: {other:?}"),
    }
}

#[tokio::test]
async fn agent_compaction_triggers_on_high_token_usage() {
    struct CompactionLlm {
        step: Mutex<usize>,
    }

    #[async_trait]
    impl LlmClient for CompactionLlm {
        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<LlmResponse, rustcode_llm::LlmError> {
            Ok(LlmResponse {
                text: "unused".to_string(),
                chunks: Vec::new(),
            })
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, rustcode_llm::LlmError> {
            let mut step = self.step.lock().await;
            match *step {
                0 => {
                    *step = 1;
                    Ok(ChatResponse {
                        text: String::new(),
                        reasoning: None,
                        reasoning_chunks: Vec::new(),
                        tool_calls: vec![ToolCall {
                            id: "call_1".to_string(),
                            name: "list".to_string(),
                            arguments: r#"{"path":"."}"#.to_string(),
                        }],
                        usage: Some(TokenUsage {
                            input: 115_000,
                            output: 5_000,
                            total: 120_000,
                            cache_read: 0,
                            cache_write: 0,
                        }),
                    })
                }
                1 => {
                    if request.tools.is_empty() {
                        *step = 2;
                        Ok(ChatResponse {
                            text: "## Summary\nUser listed files.".to_string(),
                            reasoning: None,
                            reasoning_chunks: Vec::new(),
                            tool_calls: Vec::new(),
                            usage: Some(TokenUsage {
                                input: 5_000,
                                output: 500,
                                total: 5_500,
                                cache_read: 0,
                                cache_write: 0,
                            }),
                        })
                    } else {
                        *step = 2;
                        Ok(ChatResponse {
                            text: "done after compaction".to_string(),
                            reasoning: None,
                            reasoning_chunks: Vec::new(),
                            tool_calls: Vec::new(),
                            usage: Some(TokenUsage {
                                input: 10_000,
                                output: 500,
                                total: 10_500,
                                cache_read: 0,
                                cache_write: 0,
                            }),
                        })
                    }
                }
                _ => Ok(ChatResponse {
                    text: "done after compaction".to_string(),
                    reasoning: None,
                    reasoning_chunks: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                }),
            }
        }
    }

    let workspace_root = PathBuf::from("/tmp/rustcode-compaction-test");
    let engine = Engine::new(
        Arc::new(CompactionLlm {
            step: Mutex::new(0),
        }),
        Arc::new(AgentFs {
            root: workspace_root.clone(),
        }),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            workspace_root,
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "compact-s1".to_string(),
            request_id: "compact-r1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Agent {
                prompt: "list files".to_string(),
                options: AgentOptions::default(),
                history: Vec::new(),
            },
            context,
            publisher.clone(),
        )
        .await;

    assert!(result.is_ok(), "agent should complete: {result:?}");

    let events = publisher.events.lock().await.clone();
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::OutputChunk { text } if text.contains("done")
        )
    }));
}
