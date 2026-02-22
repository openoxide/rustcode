use super::*;

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
async fn agent_persists_reasoning_when_only_reasoning_chunks_are_provided() {
    struct ChunkOnlyReasoningLlm;

    #[async_trait]
    impl LlmClient for ChunkOnlyReasoningLlm {
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
            _request: ChatRequest,
        ) -> Result<ChatResponse, rustcode_llm::LlmError> {
            Ok(ChatResponse {
                text: "done".to_string(),
                reasoning: None,
                reasoning_chunks: vec!["step 1".to_string(), " + step 2".to_string()],
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    #[derive(Default)]
    struct CapturingRecorder {
        messages: Mutex<Vec<StoredMessage>>,
    }

    #[async_trait]
    impl TranscriptRecorder for CapturingRecorder {
        async fn append_message(
            &self,
            _session_id: &str,
            message: StoredMessage,
        ) -> Result<(), ExecutionError> {
            self.messages.lock().await.push(message);
            Ok(())
        }
    }

    let recorder = Arc::new(CapturingRecorder::default());
    let engine = Engine::new(
        Arc::new(ChunkOnlyReasoningLlm),
        Arc::new(DummyFs),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        Some(recorder.clone()),
        None,
    );
    let publisher = Arc::new(CollectingPublisher::default());
    let context = CommandContext::new(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "reasoning-s1".to_string(),
            request_id: "reasoning-r1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let result = engine
        .execute(
            Command::Agent {
                prompt: "reason please".to_string(),
                options: AgentOptions::default(),
                history: Vec::new(),
            },
            context,
            publisher,
        )
        .await;
    assert!(result.is_ok(), "result={result:?}");

    let recorded = recorder.messages.lock().await.clone();
    let assistant = recorded
        .iter()
        .find(|msg| msg.role == MessageRole::Assistant)
        .expect("assistant message should be recorded");
    assert_eq!(assistant.reasoning.as_deref(), Some("step 1 + step 2"));
}
