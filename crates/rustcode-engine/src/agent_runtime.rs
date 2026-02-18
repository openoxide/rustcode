use super::*;

impl Engine {
    pub(crate) async fn run_agent(
        &self,
        prompt: String,
        options: AgentOptions,
        history: Vec<StoredMessage>,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let mut tools =
            agent_tools::AgentToolRegistry::tool_specs(&options, context.config.allow_network);

        if context.config.allow_network {
            if let Some(mcp) = self.mcp.as_ref() {
                match mcp.tool_specs().await {
                    Ok(mcp_tools) => {
                        for (_namespaced_name, spec) in mcp_tools {
                            tools.push(spec);
                        }
                    }
                    Err(err) => {
                        tracing::warn!("failed to fetch MCP tool specs: {err}");
                        self.emit(
                            publisher.clone(),
                            EventScope::System,
                            EventPayload::Warning {
                                message: format!("mcp tools unavailable: {err}"),
                            },
                            context,
                        )
                        .await?;
                    }
                }
            }
        }

        let mut state = AgentState::default();

        let system_prompt = "You are rustcode, a production-grade coding agent.\n\
Use tools when you need filesystem context.\n\
Prefer: list -> read.\n\
Only modify files via write/edit when explicitly required.\n\
When you are done, respond with a final plain-text answer."
            .to_string();

        let mut messages = Vec::new();
        if history.is_empty() {
            let system_id = self.new_message_id();
            self.record_message(
                context,
                StoredMessage {
                    id: system_id,
                    role: MessageRole::System,
                    created_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                    content: Value::String(system_prompt.clone()),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                },
            )
            .await?;
            messages.push(ChatMessage {
                role: ChatRole::System,
                content: Value::String(system_prompt),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            });
        } else {
            messages.extend(stored_messages_to_chat(history));
            if !matches!(messages.first().map(|m| m.role), Some(ChatRole::System)) {
                messages.insert(
                    0,
                    ChatMessage {
                        role: ChatRole::System,
                        content: Value::String(system_prompt),
                        tool_call_id: None,
                        tool_name: None,
                        tool_calls: Vec::new(),
                    },
                );
            }
        }

        let user_id = self.new_message_id();
        self.record_message(
            context,
            StoredMessage {
                id: user_id.clone(),
                role: MessageRole::User,
                created_at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
                content: Value::String(prompt.clone()),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
        )
        .await?;

        messages.push(ChatMessage {
            role: ChatRole::User,
            content: Value::String(prompt),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        });

        for _step in 0..options.max_steps {
            let request = ChatRequest {
                model: context.config.model.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                initiator: RequestInitiator::Agent,
            };

            let response = tokio::select! {
                _ = context.cancellation.cancelled() => {
                    return Err(ExecutionError::Cancelled);
                }
                result = self.llm.chat(request) => {
                    result
                }
            }
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

            messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: if response.text.is_empty() {
                    Value::Null
                } else {
                    Value::String(response.text.clone())
                },
                tool_call_id: None,
                tool_name: None,
                tool_calls: response.tool_calls.clone(),
            });

            let assistant_id = self.new_message_id();
            self.record_message(
                context,
                StoredMessage {
                    id: assistant_id,
                    role: MessageRole::Assistant,
                    created_at_unix_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                    content: if response.text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(response.text.clone())
                    },
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: response
                        .tool_calls
                        .iter()
                        .map(|call| StoredToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        })
                        .collect(),
                },
            )
            .await?;

            if !response.text.is_empty() {
                self.emit(
                    publisher.clone(),
                    EventScope::Command,
                    EventPayload::OutputChunk {
                        text: response.text.clone(),
                    },
                    context,
                )
                .await?;
            }

            if response.tool_calls.is_empty() {
                return Ok(());
            }

            let calls = response
                .tool_calls
                .iter()
                .take(options.max_tool_calls_per_step)
                .cloned()
                .collect::<Vec<_>>();

            for call in &calls {
                self.emit(
                    publisher.clone(),
                    EventScope::Tool,
                    EventPayload::ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                    context,
                )
                .await?;
            }

            let can_parallelize = calls.len() > 1
                && calls
                    .iter()
                    .all(|call| is_parallel_safe_tool(call.name.as_str()));

            let results = if can_parallelize {
                join_all(calls.iter().map(|call| async {
                    let mut local_state = AgentState::default();
                    (
                        call.id.clone(),
                        call.name.clone(),
                        self.execute_agent_tool_call(
                            call.name.as_str(),
                            call.arguments.as_str(),
                            context,
                            &options,
                            &mut local_state,
                        )
                        .await,
                    )
                }))
                .await
            } else {
                let mut out = Vec::with_capacity(calls.len());
                for call in &calls {
                    out.push((
                        call.id.clone(),
                        call.name.clone(),
                        self.execute_agent_tool_call(
                            call.name.as_str(),
                            call.arguments.as_str(),
                            context,
                            &options,
                            &mut state,
                        )
                        .await,
                    ));
                }
                out
            };

            for (call, (id, name, tool_result)) in calls.iter().zip(results.into_iter()) {
                debug_assert_eq!(call.id, id);
                debug_assert_eq!(call.name, name);

                let (ok, output) = match tool_result {
                    Ok(output) => (true, output),
                    Err(ExecutionError::Dispatch(message)) => (false, message),
                    Err(ExecutionError::Executor(message)) => (false, message),
                    Err(err) => (false, err.to_string()),
                };
                let result_payload = tool_payload_json(ok, output, options.max_tool_result_bytes);

                self.emit(
                    publisher.clone(),
                    EventScope::Tool,
                    EventPayload::ToolResult {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        ok,
                        output: result_payload.clone(),
                    },
                    context,
                )
                .await?;

                let tool_msg_id = self.new_message_id();
                self.record_message(
                    context,
                    StoredMessage {
                        id: tool_msg_id,
                        role: MessageRole::Tool,
                        created_at_unix_ms: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0),
                        content: Value::String(result_payload.clone()),
                        tool_call_id: Some(call.id.clone()),
                        tool_name: Some(call.name.clone()),
                        tool_calls: Vec::new(),
                    },
                )
                .await?;

                messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: Value::String(result_payload),
                    tool_call_id: Some(call.id.clone()),
                    tool_name: Some(call.name.clone()),
                    tool_calls: Vec::new(),
                });
            }
        }

        Err(ExecutionError::Executor(
            "agent exceeded maximum tool loop steps".to_string(),
        ))
    }

    pub(crate) async fn execute_agent_tool_call(
        &self,
        name: &str,
        arguments: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        if context.cancellation.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        let args: Value = serde_json::from_str(arguments).map_err(|err| {
            ExecutionError::Dispatch(format!("tool arguments are not valid JSON: {err}"))
        })?;

        if is_mutating_tool(name) {
            let (permission, pattern, reason) = approval_fields(name, &args);
            let match_targets = approval_match_targets(name, &args);
            let decision = resolve_permission_action(
                &context.config.permission_rules,
                &permission,
                &match_targets,
            )?;

            match decision {
                Some(PermissionAction::Deny) => {
                    return Err(ExecutionError::Dispatch(format!(
                        "tool permission denied by rule: tool={name} permission={permission} target={pattern}"
                    )));
                }
                Some(PermissionAction::Allow) => {}
                Some(PermissionAction::Ask) | None => {
                    let Some(approver) = self.approver.as_ref() else {
                        return Err(ExecutionError::Dispatch(format!(
                            "tool approval required but no interactive approver is available: tool={name} permission={permission} target={pattern}"
                        )));
                    };
                    let approved = approver
                        .approve(ToolApprovalRequest {
                            tool: name.to_string(),
                            permission,
                            pattern,
                            arguments: args.clone(),
                            reason,
                        })
                        .await?;
                    if !approved {
                        return Err(ExecutionError::Dispatch(
                            "tool execution rejected by user".to_string(),
                        ));
                    }
                }
            }
        }

        if name.starts_with("mcp:") {
            if let Some(mcp) = self.mcp.as_ref() {
                match mcp.call_tool(name, args).await {
                    Ok(result) => return Ok(result),
                    Err(err) => {
                        return Err(ExecutionError::Executor(format!("MCP tool failed: {err}")));
                    }
                }
            } else {
                return Err(ExecutionError::Dispatch(
                    "MCP tool called but MCP registry not configured".to_string(),
                ));
            }
        }

        agent_tools::AgentToolRegistry::execute(self, name, args, context, options, state).await
    }
}

fn stored_messages_to_chat(history: Vec<StoredMessage>) -> Vec<ChatMessage> {
    history
        .into_iter()
        .map(|msg| ChatMessage {
            role: match msg.role {
                MessageRole::System => ChatRole::System,
                MessageRole::User => ChatRole::User,
                MessageRole::Assistant => ChatRole::Assistant,
                MessageRole::Tool => ChatRole::Tool,
            },
            content: msg.content,
            tool_call_id: msg.tool_call_id,
            tool_name: msg.tool_name,
            tool_calls: msg
                .tool_calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments,
                })
                .collect(),
        })
        .collect()
}

fn tool_payload_json(ok: bool, output: String, max_bytes: usize) -> String {
    let mut truncated = false;
    let mut candidate_output = output;

    for _ in 0..8 {
        let payload = json!({
            "ok": ok,
            "truncated": truncated,
            "output": candidate_output,
        })
        .to_string();

        if payload.len() <= max_bytes {
            return payload;
        }

        truncated = true;
        let limit = max_bytes.saturating_sub(256);
        candidate_output = truncate_utf8_bytes(&candidate_output, limit);
    }

    json!({
        "ok": ok,
        "truncated": true,
        "output": "[tool output truncated]".to_string(),
    })
    .to_string()
}

fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = 0usize;
    for (idx, _) in input.char_indices() {
        if idx > max_bytes {
            break;
        }
        end = idx;
    }
    let prefix = &input[..end];
    format!("{prefix}\n...[truncated]...\n")
}

fn resolve_permission_action(
    rules: &[rustcode_core::PermissionRule],
    permission: &str,
    targets: &[String],
) -> Result<Option<PermissionAction>, ExecutionError> {
    let normalized_targets = targets
        .iter()
        .map(|target| target.replace('\\', "/"))
        .collect::<Vec<_>>();

    for rule in rules.iter().rev() {
        if !rule.permission.eq_ignore_ascii_case(permission) {
            continue;
        }
        let matcher = Glob::new(&rule.pattern)
            .map_err(|err| {
                ExecutionError::Dispatch(format!(
                    "invalid permissions glob pattern {}: {err}",
                    rule.pattern
                ))
            })?
            .compile_matcher();

        if normalized_targets
            .iter()
            .any(|target| !target.is_empty() && matcher.is_match(target))
        {
            return Ok(Some(rule.action));
        }
    }

    Ok(None)
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write" | "edit" | "exec") || name.starts_with("mcp:")
}

fn is_parallel_safe_tool(name: &str) -> bool {
    matches!(name, "list" | "glob" | "grep" | "webfetch" | "todowrite")
}

fn approval_fields(tool: &str, args: &Value) -> (String, String, String) {
    let permission = if tool.starts_with("mcp:") {
        "mcp".to_string()
    } else {
        tool.to_string()
    };
    let (pattern, reason) = match tool {
        "write" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to write {path}"),
            )
        }
        "edit" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to edit {path}"),
            )
        }
        "exec" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let mut rendered = command.to_string();
            if let Some(items) = args.get("args").and_then(Value::as_array) {
                for item in items.iter().filter_map(Value::as_str) {
                    rendered.push(' ');
                    rendered.push_str(item);
                }
            }
            (
                rendered.clone(),
                format!("agent requests permission to execute {rendered}"),
            )
        }
        _ => (
            tool.to_string(),
            format!("agent requests permission to call MCP tool {tool}"),
        ),
    };

    (permission, pattern, reason)
}

fn approval_match_targets(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        _ if tool.starts_with("mcp:") => vec![tool.to_string()],
        "exec" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut targets = Vec::new();
            if !command.is_empty() {
                targets.push(command.clone());
            }
            if let Some(items) = args.get("args").and_then(Value::as_array) {
                let mut rendered = command.clone();
                for item in items.iter().filter_map(Value::as_str) {
                    if !rendered.is_empty() {
                        rendered.push(' ');
                    }
                    rendered.push_str(item);
                }
                if !rendered.is_empty() && rendered != command {
                    targets.push(rendered);
                }
            }
            if targets.is_empty() {
                targets.push(String::new());
            }
            targets
        }
        "write" | "edit" => vec![args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        _ => vec!["*".to_string()],
    }
}
