use super::fixtures::*;
use super::*;

#[tokio::test]
async fn mutating_tools_require_approval_when_no_allow_rule_and_no_approver() {
    let mut cfg = ResolvedConfig::default();
    cfg.permission_rules.clear();
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-1".to_string(),
            request_id: "r-approve-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "ok".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 1024,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo","args":["hi"]}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    match result {
        Err(ExecutionError::Dispatch(message)) => {
            assert!(message.contains("approval required"), "message={message}");
        }
        other => panic!("expected dispatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn mcp_tools_require_approval_when_no_allow_rule_and_no_approver() {
    let cfg = ResolvedConfig::default();
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-mcp-1".to_string(),
            request_id: "r-approve-mcp-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "ok".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );

    let options = AgentOptions::default();
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call("mcp:demo:tool", r"{}", &context, &options, &mut state)
        .await;
    match result {
        Err(ExecutionError::Dispatch(message)) => {
            assert!(message.contains("approval required"), "message={message}");
        }
        other => panic!("expected dispatch error, got {other:?}"),
    }
}

#[tokio::test]
async fn mutating_tools_can_be_allowed_by_permissions_without_prompt() {
    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Allow,
            pattern: "echo".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-2".to_string(),
            request_id: "r-approve-2".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hello".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 4096,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo","args":["hi"]}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("exec should be allowed");
    assert!(output.contains("exit_code=0"), "output={output}");
}

#[tokio::test]
async fn exec_permission_rule_can_match_full_command_line() {
    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Allow,
            pattern: "echo hi".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-exec-full-1".to_string(),
            request_id: "r-approve-exec-full-1".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hello".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        None,
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 4096,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo","args":["hi"]}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("exec should be allowed");
    assert!(output.contains("exit_code=0"), "output={output}");
}

#[tokio::test]
async fn exec_permission_rule_precedence_prefers_last_match_across_targets() {
    let cfg = ResolvedConfig {
        permission_rules: vec![
            rustcode_core::PermissionRule {
                permission: "exec".to_string(),
                action: PermissionAction::Allow,
                pattern: "echo".to_string(),
            },
            rustcode_core::PermissionRule {
                permission: "exec".to_string(),
                action: PermissionAction::Deny,
                pattern: "echo hi".to_string(),
            },
        ],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-exec-full-2".to_string(),
            request_id: "r-approve-exec-full-2".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let approver = Arc::new(CountingApprover {
        approved: true,
        calls: AtomicUsize::new(0),
    });
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hello".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        Some(approver.clone()),
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 1024,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo","args":["hi"]}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    assert!(matches!(result, Err(ExecutionError::Dispatch(_))));
    assert_eq!(approver.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn mutating_tools_can_be_denied_by_permissions_without_prompt() {
    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Deny,
            pattern: "echo".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-3".to_string(),
            request_id: "r-approve-3".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let approver = Arc::new(CountingApprover {
        approved: true,
        calls: AtomicUsize::new(0),
    });
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hello".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        Some(approver.clone()),
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 1024,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let result = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo"}"#,
            &context,
            &options,
            &mut state,
        )
        .await;
    assert!(matches!(result, Err(ExecutionError::Dispatch(_))));
    assert_eq!(approver.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn mutating_tools_ask_rule_triggers_approver() {
    let cfg = ResolvedConfig {
        permission_rules: vec![rustcode_core::PermissionRule {
            permission: "exec".to_string(),
            action: PermissionAction::Ask,
            pattern: "echo".to_string(),
        }],
        ..ResolvedConfig::default()
    };
    let context = CommandContext::new(
        Arc::new(cfg),
        SessionMeta {
            session_id: "s-approve-4".to_string(),
            request_id: "r-approve-4".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let approver = Arc::new(CountingApprover {
        approved: true,
        calls: AtomicUsize::new(0),
    });
    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(DummyFs),
        Arc::new(StubProcess {
            stdout: "hello".to_string(),
            stderr: String::new(),
            code: 0,
        }),
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        None,
        Some(approver.clone()),
    );

    let options = AgentOptions {
        max_steps: 1,
        max_tool_calls_per_step: 1,
        allow_write: false,
        allow_edit: false,
        allow_exec: true,
        max_read_bytes: 1024,
        max_list_entries: 100,
        max_tool_result_bytes: 4096,
        max_write_bytes: 1024,
        mode_hint: None,
    };
    let mut state = AgentState::default();
    let output = engine
        .execute_agent_tool_call(
            "exec",
            r#"{"command":"echo"}"#,
            &context,
            &options,
            &mut state,
        )
        .await
        .expect("exec should be approved");
    assert!(output.contains("exit_code=0"), "output={output}");
    assert_eq!(approver.calls.load(Ordering::Relaxed), 1);
}
