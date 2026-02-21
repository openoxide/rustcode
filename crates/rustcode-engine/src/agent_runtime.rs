use super::{
    agent_tools, join_all, AgentOptions, AgentState, Arc, ChatMessage, ChatRequest, ChatRole,
    CommandContext, Engine, EventPayload, EventPublisher, EventScope, ExecutionError, MessageRole,
    PermissionAction, RequestInitiator, StoredMessage, StoredToolCall, SystemTime,
    ToolApprovalRequest, ToolCall, Value, UNIX_EPOCH,
};

use crate::agent_util::{
    approval_fields, approval_match_targets, is_mutating_tool, is_parallel_safe_tool,
    resolve_permission_action, stored_messages_to_chat, tool_payload_json,
};
use crate::context_tracker::ContextTracker;

impl Engine {
    #[tracing::instrument(skip_all, fields(model = %context.config.model, steps = options.max_steps))]
    pub(crate) async fn run_agent(
        &self,
        prompt: String,
        options: AgentOptions,
        history: Vec<StoredMessage>,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let mut tools = agent_tools::AgentToolRegistry::tool_specs(
            &options,
            context.config.allow_network,
            self.lsp_manager.is_some(),
        );

        if context.config.allow_network {
            tokio::select! {
                () = context.cancellation.cancelled() => return Err(ExecutionError::Cancelled),
                result = self.load_mcp_tools(&mut tools, publisher.clone(), context) => result?,
            }
        }

        let mut state = AgentState::default();

        // Load memory summary if available
        let memory_summary = self
            .memories
            .as_ref()
            .and_then(|m| m.load_summary())
            .map(|s| s.content);

        let system_prompt = crate::system_prompt::build_system_prompt(
            &context.config.model,
            &context.config.workspace_root,
            crate::system_prompt::is_git_repo(&context.config.workspace_root),
            &tools,
            self.skills.all(),
            memory_summary.as_deref(),
        );

        let mut messages = self
            .build_initial_messages(&system_prompt, &prompt, history, context)
            .await?;

        let mut context_tracker = ContextTracker::new(&context.config.model);

        for step in 0..options.max_steps {
            tracing::debug!(step, "agent step");
            // Check for context overflow and compact if needed
            if context_tracker.is_overflow() {
                tracing::info!(
                    "context overflow detected ({}), compacting",
                    context_tracker.status_line()
                );
                if let Err(err) = self.compact_context(&mut messages, context).await {
                    tracing::warn!("compaction failed, continuing with pruned context: {err}");
                }
                // Reset tracker after compaction (next LLM step will report new usage)
                context_tracker = ContextTracker::new(&context.config.model);
            }

            let retry_policy = crate::retry::RetryPolicy::default();
            let retry_publisher = publisher.clone();
            let on_retry = move |msg: &str| {
                let _ = retry_publisher.publish(rustcode_core::event::Event::new(
                    0,
                    rustcode_core::event::EventScope::Command,
                    rustcode_core::event::EventPayload::Warning {
                        message: msg.to_string(),
                    },
                ));
            };
            let response = crate::retry::retry_llm_call(
                &retry_policy,
                || self.run_llm_step(&messages, &tools, context),
                Some(&on_retry),
            )
            .await?;

            // Record token usage for context tracking
            if let Some(usage) = &response.usage {
                context_tracker.record_usage(usage);
                tracing::debug!("context: {}", context_tracker.status_line());
                // Publish usage to TUI for live display
                let _ = publisher
                    .publish(rustcode_core::event::Event::new(
                        0,
                        rustcode_core::event::EventScope::Command,
                        rustcode_core::event::EventPayload::UsageUpdate {
                            input_tokens: usage.input,
                            output_tokens: usage.output,
                            total_tokens: usage.total,
                            cache_read: usage.cache_read,
                            cache_write: usage.cache_write,
                            context_limit: context_tracker.context_limit(),
                        },
                    ))
                    .await;
            }

            self.record_assistant_response(&response, &messages, publisher.clone(), context)
                .await?;

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

            if !response.reasoning_chunks.is_empty() {
                for chunk in &response.reasoning_chunks {
                    if chunk.trim().is_empty() {
                        continue;
                    }
                    self.emit(
                        publisher.clone(),
                        EventScope::Command,
                        EventPayload::ReasoningChunk {
                            text: chunk.clone(),
                        },
                        context,
                    )
                    .await?;
                }
            } else if let Some(reasoning) = response.reasoning.as_ref() {
                if !reasoning.trim().is_empty() {
                    self.emit(
                        publisher.clone(),
                        EventScope::Command,
                        EventPayload::ReasoningChunk {
                            text: reasoning.clone(),
                        },
                        context,
                    )
                    .await?;
                }
            }

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

            self.execute_tool_calls(
                &response.tool_calls,
                &options,
                context,
                &mut state,
                publisher.clone(),
                &mut messages,
            )
            .await?;
        }

        Err(ExecutionError::Executor(
            "agent exceeded maximum tool loop steps".to_string(),
        ))
    }

    async fn load_mcp_tools(
        &self,
        tools: &mut Vec<rustcode_llm::ToolSpec>,
        publisher: Arc<dyn EventPublisher>,
        context: &CommandContext,
    ) -> Result<(), ExecutionError> {
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
                        publisher,
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
        Ok(())
    }

    async fn build_initial_messages(
        &self,
        system_prompt: &str,
        prompt: &str,
        history: Vec<StoredMessage>,
        context: &CommandContext,
    ) -> Result<Vec<ChatMessage>, ExecutionError> {
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
                    content: Value::String(system_prompt.to_string()),
                    reasoning: None,
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                },
            )
            .await?;
            messages.push(ChatMessage {
                role: ChatRole::System,
                content: Value::String(system_prompt.to_string()),
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
                        content: Value::String(system_prompt.to_string()),
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
                content: Value::String(prompt.to_string()),
                reasoning: None,
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
        )
        .await?;

        messages.push(ChatMessage {
            role: ChatRole::User,
            content: Value::String(prompt.to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        });

        Ok(messages)
    }

    #[tracing::instrument(skip_all, fields(model = %context.config.model, messages = messages.len(), tools = tools.len()))]
    async fn run_llm_step(
        &self,
        messages: &[ChatMessage],
        tools: &[rustcode_llm::ToolSpec],
        context: &CommandContext,
    ) -> Result<rustcode_llm::ChatResponse, ExecutionError> {
        let request = ChatRequest {
            model: context.config.model.clone(),
            messages: messages.to_vec(),
            tools: tools.to_vec(),
            initiator: RequestInitiator::Agent,
        };

        tokio::select! {
            () = context.cancellation.cancelled() => {
                Err(ExecutionError::Cancelled)
            }
            result = self.llm.chat(request) => {
                result.map_err(|err| ExecutionError::Executor(err.to_string()))
            }
        }
    }

    async fn record_assistant_response(
        &self,
        response: &rustcode_llm::ChatResponse,
        _messages: &[ChatMessage],
        _publisher: Arc<dyn EventPublisher>,
        context: &CommandContext,
    ) -> Result<(), ExecutionError> {
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
                reasoning: response.reasoning.clone(),
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
        .await
    }

    #[tracing::instrument(skip_all, fields(n = all_calls.len()))]
    async fn execute_tool_calls(
        &self,
        all_calls: &[ToolCall],
        options: &AgentOptions,
        context: &CommandContext,
        state: &mut AgentState,
        publisher: Arc<dyn EventPublisher>,
        messages: &mut Vec<ChatMessage>,
    ) -> Result<(), ExecutionError> {
        let calls = all_calls
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
            let futs = join_all(calls.iter().map(|call| async {
                let mut local_state = AgentState::default();
                (
                    call.id.clone(),
                    call.name.clone(),
                    self.execute_agent_tool_call(
                        call.name.as_str(),
                        call.arguments.as_str(),
                        context,
                        options,
                        &mut local_state,
                    )
                    .await,
                )
            }));
            tokio::select! {
                () = context.cancellation.cancelled() => return Err(ExecutionError::Cancelled),
                results = futs => results,
            }
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
                        options,
                        state,
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
                    reasoning: None,
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

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(tool = %name))]
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

        if is_mutating_tool(name, &args) {
            self.check_tool_permission(name, &args, context).await?;
        }

        tracing::debug!(tool = %name, "executing tool");

        if name.starts_with("mcp:") {
            return self.execute_mcp_tool(name, args).await;
        }

        agent_tools::AgentToolRegistry::execute(self, name, args, context, options, state).await
    }

    async fn check_tool_permission(
        &self,
        name: &str,
        args: &Value,
        context: &CommandContext,
    ) -> Result<(), ExecutionError> {
        let (permission, pattern, reason) = approval_fields(name, args);
        let match_targets = approval_match_targets(name, args);
        let decision = resolve_permission_action(
            &context.config.permission_rules,
            &permission,
            &match_targets,
        )?;

        match decision {
            Some(PermissionAction::Deny) => Err(ExecutionError::Dispatch(format!(
                "tool permission denied by rule: tool={name} permission={permission} target={pattern}"
            ))),
            Some(PermissionAction::Allow) => Ok(()),
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
                Ok(())
            }
        }
    }

    async fn execute_mcp_tool(&self, name: &str, args: Value) -> Result<String, ExecutionError> {
        if let Some(mcp) = self.mcp.as_ref() {
            match mcp.call_tool(name, args).await {
                Ok(result) => Ok(result),
                Err(err) => Err(ExecutionError::Executor(format!("MCP tool failed: {err}"))),
            }
        } else {
            Err(ExecutionError::Dispatch(
                "MCP tool called but MCP registry not configured".to_string(),
            ))
        }
    }
}
