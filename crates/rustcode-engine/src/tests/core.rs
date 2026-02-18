use super::fixtures::*;
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

#[tokio::test]
async fn agent_executes_tool_calls_and_emits_tool_events() {
    let workspace_root = PathBuf::from("/tmp/rustcode-agent-workspace");
    let engine = Engine::new(
        Arc::new(ScriptedAgentLlm {
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
            session_id: "agent-s1".to_string(),
            request_id: "agent-r1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Agent {
                prompt: "hi".to_string(),
                options: AgentOptions::default(),
                history: Vec::new(),
            },
            context,
            publisher.clone(),
        )
        .await;
    assert!(result.is_ok(), "result={result:?}");

    let events = publisher.events.lock().await.clone();
    assert!(events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::ToolCall { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::ToolResult { .. })));
    assert!(events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::OutputChunk { ref text } if text.contains("done"))));
}

#[tokio::test]
async fn agent_can_exec_when_allowed() {
    struct ExecAgentLlm {
        step: Mutex<usize>,
    }

    #[async_trait]
    impl LlmClient for ExecAgentLlm {
        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<LlmResponse, rustcode_llm::LlmError> {
            Ok(LlmResponse {
                text: "unused".to_string(),
                chunks: Vec::new(),
            })
        }

        async fn chat(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, rustcode_llm::LlmError> {
            let mut step = self.step.lock().await;
            match *step {
                0 => {
                    *step = 1;
                    assert!(
                        request.tools.iter().any(|tool| tool.name == "exec"),
                        "expected exec tool spec when allow-exec is enabled"
                    );
                    Ok(ChatResponse {
                        text: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call_exec".to_string(),
                            name: "exec".to_string(),
                            arguments: r#"{"command":"echo","args":["hi"]}"#.to_string(),
                        }],
                        usage: None,
                    })
                }
                _ => {
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|msg| msg.tool_call_id.as_deref() == Some("call_exec")),
                        "expected tool result message for call_exec"
                    );
                    Ok(ChatResponse {
                        text: "done".to_string(),
                        tool_calls: Vec::new(),
                        usage: None,
                    })
                }
            }
        }
    }

    let workspace_root = PathBuf::from("/tmp/rustcode-agent-exec-workspace");
    let engine = Engine::new(
        Arc::new(ExecAgentLlm {
            step: Mutex::new(0),
        }),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hi\n".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            workspace_root,
            permission_rules: vec![rustcode_core::PermissionRule {
                permission: "exec".to_string(),
                action: PermissionAction::Allow,
                pattern: "echo".to_string(),
            }],
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "agent-exec-s1".to_string(),
            request_id: "agent-exec-r1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Agent {
                prompt: "hi".to_string(),
                options: AgentOptions {
                    allow_exec: true,
                    ..AgentOptions::default()
                },
                history: Vec::new(),
            },
            context,
            publisher.clone(),
        )
        .await;
    assert!(result.is_ok(), "result={result:?}");

    let events = publisher.events.lock().await.clone();
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::ToolCall { name, .. } if name == "exec"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::ToolResult { name, ok, output, .. }
            if name == "exec" && *ok && output.contains("exit_code=0")
        )
    }));
}

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

#[tokio::test]
async fn run_prompt_emits_streaming_chunks_when_available() {
    let engine = Engine::new(
        Arc::new(StreamingLlmClient),
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
            session_id: "s-stream".to_string(),
            request_id: "r-stream".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Run {
                prompt: "stream".to_string(),
            },
            context,
            publisher.clone(),
        )
        .await;
    assert!(result.is_ok());

    let events = publisher.events.lock().await.clone();
    let chunks: Vec<String> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::OutputChunk { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(chunks, vec!["hello".to_string(), " world".to_string()]);
}

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
