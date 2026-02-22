use super::*;

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

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, rustcode_llm::LlmError> {
            let mut step = self.step.lock().await;
            if *step == 0 {
                *step = 1;
                assert!(
                    request.tools.iter().any(|tool| tool.name == "exec"),
                    "expected exec tool spec when allow-exec is enabled"
                );
                Ok(ChatResponse {
                    text: String::new(),
                    reasoning: None,
                    reasoning_chunks: Vec::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_exec".to_string(),
                        name: "exec".to_string(),
                        arguments: r#"{"command":"echo","args":["hi"]}"#.to_string(),
                    }],
                    usage: None,
                })
            } else {
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|msg| msg.tool_call_id.as_deref() == Some("call_exec")),
                    "expected tool result message for call_exec"
                );
                Ok(ChatResponse {
                    text: "done".to_string(),
                    reasoning: None,
                    reasoning_chunks: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: None,
                })
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
