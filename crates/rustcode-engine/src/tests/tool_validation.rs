use super::fixtures::*;
use super::*;

#[tokio::test]
async fn glob_returns_matching_files_relative_to_workspace() {
    let workspace_root = PathBuf::from("/tmp/rustcode-glob-workspace");
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(SearchFs {
            root: workspace_root.clone(),
        }),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            workspace_root: workspace_root.clone(),
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-glob-1".to_string(),
            request_id: "r-glob-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "glob",
            r#"{"pattern":"*.txt"}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("glob should succeed");

    assert!(output.contains("a.txt"), "output={output}");
    assert!(!output.contains("b.md"), "output={output}");
    assert!(!output.contains("dir"), "output={output}");
}

#[tokio::test]
async fn grep_respects_include_filter_and_emits_line_matches() {
    let workspace_root = PathBuf::from("/tmp/rustcode-grep-workspace");
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(SearchFs {
            root: workspace_root.clone(),
        }),
        Arc::new(CancelledProcess),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );
    let context = CommandContext::new(
        Arc::new(ResolvedConfig {
            workspace_root: workspace_root.clone(),
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-grep-1".to_string(),
            request_id: "r-grep-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions {
        max_read_bytes: 64 * 1024,
        max_list_entries: 100,
        ..AgentOptions::default()
    };
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "grep",
            r#"{"pattern":"needle","include":"*.txt"}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("grep should succeed");

    assert!(output.contains("a.txt:2: needle here"), "output={output}");
    assert!(!output.contains("b.md"), "output={output}");
}

#[tokio::test]
async fn tool_rejects_unknown_argument_keys() {
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
            workspace_root: PathBuf::from("/tmp/rustcode-unknown-args"),
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-args-1".to_string(),
            request_id: "r-args-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "read",
            r#"{"path":"README.md","extra":true}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    match result {
        Err(ExecutionError::Dispatch(message)) => {
            assert!(
                message.contains("unexpected argument key"),
                "message={message}"
            );
        }
        other => panic!("expected dispatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn tool_rejects_wrong_argument_types() {
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
            workspace_root: PathBuf::from("/tmp/rustcode-bad-types"),
            allow_network: true,
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-args-2".to_string(),
            request_id: "r-args-2".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "webfetch",
            r#"{"url":"https://example.com","timeout_secs":"nope"}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    match result {
        Err(ExecutionError::Dispatch(message)) => {
            assert!(message.contains("timeout_secs"), "message={message}");
        }
        other => panic!("expected dispatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn todowrite_accepts_and_normalizes_todos() {
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
            workspace_root: PathBuf::from("/tmp/rustcode-todo"),
            ..ResolvedConfig::default()
        }),
        SessionMeta {
            session_id: "s-todo-1".to_string(),
            request_id: "r-todo-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "todowrite",
            r#"{"todos":[{"content":"  do thing  ","status":"PENDING","priority":"High"}]}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("todowrite should succeed");
    assert!(output.contains("\"todos\""), "output={output}");
    assert!(output.contains("\"pending\""), "output={output}");
    assert!(output.contains("\"high\""), "output={output}");
    assert!(output.contains("do thing"), "output={output}");
}
