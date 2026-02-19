use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use rustcode_core::command::AgentOptions;
use rustcode_core::context::CommandContext;
use rustcode_core::error::ExecutionError;
use rustcode_llm::ToolSpec;

use crate::agent_handlers_multiedit::MultiEditOp;
use crate::{AgentState, Engine};

pub struct AgentToolRegistry;

fn ensure_allowed_keys(args: &Value, allowed: &[&str]) -> Result<(), ExecutionError> {
    let obj = args.as_object().ok_or_else(|| {
        ExecutionError::Dispatch("tool arguments must be a JSON object".to_string())
    })?;
    for key in obj.keys() {
        if !allowed.iter().any(|allowed_key| key == *allowed_key) {
            return Err(ExecutionError::Dispatch(format!(
                "unexpected argument key: {key}"
            )));
        }
    }
    Ok(())
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ExecutionError> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ExecutionError::Dispatch(format!("{key} must be a string"))),
    }
}

fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, ExecutionError> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Number(num)) => num
            .as_u64()
            .ok_or_else(|| ExecutionError::Dispatch(format!("{key} must be an integer")))
            .map(Some),
        Some(_) => Err(ExecutionError::Dispatch(format!(
            "{key} must be an integer"
        ))),
    }
}

fn opt_str_list(args: &Value, key: &str) -> Result<Vec<String>, ExecutionError> {
    match args.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(value) = item.as_str() else {
                    return Err(ExecutionError::Dispatch(format!(
                        "{key} must be an array of strings"
                    )));
                };
                out.push(value.to_string());
            }
            Ok(out)
        }
        Some(_) => Err(ExecutionError::Dispatch(format!(
            "{key} must be an array of strings"
        ))),
    }
}

impl AgentToolRegistry {
    pub fn tool_specs(options: &AgentOptions, allow_network: bool) -> Vec<ToolSpec> {
        super::agent_tool_specs::tool_specs(options, allow_network)
    }

    pub fn execute<'a>(
        engine: &'a Engine,
        name: &'a str,
        args: Value,
        context: &'a CommandContext,
        options: &'a AgentOptions,
        state: &'a mut AgentState,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ExecutionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            match name {
                "list" => {
                    ensure_allowed_keys(&args, &["path"])?;
                    let path = opt_str(&args, "path")?.map(std::string::ToString::to_string);
                    engine.agent_tool_list(path, context, options).await
                }
                "read" => {
                    ensure_allowed_keys(&args, &["path"])?;
                    let path = opt_str(&args, "path")?.ok_or_else(|| {
                        ExecutionError::Dispatch("read tool requires path".to_string())
                    })?;
                    engine.agent_tool_read(path, context, options, state).await
                }
                "write" => {
                    ensure_allowed_keys(&args, &["path", "contents"])?;
                    if !options.allow_write && !options.allow_edit {
                        return Err(ExecutionError::Dispatch(
                            "agent write is disabled; rerun with --allow-write".to_string(),
                        ));
                    }
                    let path = opt_str(&args, "path")?.ok_or_else(|| {
                        ExecutionError::Dispatch("write tool requires path".to_string())
                    })?;
                    let contents = opt_str(&args, "contents")?.ok_or_else(|| {
                        ExecutionError::Dispatch("write tool requires contents".to_string())
                    })?;
                    engine
                        .agent_tool_write(path, contents, context, options, state)
                        .await
                }
                "edit" => {
                    ensure_allowed_keys(&args, &["path", "from", "to"])?;
                    if !options.allow_edit {
                        return Err(ExecutionError::Dispatch(
                            "agent edit is disabled; rerun with --allow-edit".to_string(),
                        ));
                    }
                    let path = opt_str(&args, "path")?.ok_or_else(|| {
                        ExecutionError::Dispatch("edit tool requires path".to_string())
                    })?;
                    let from = opt_str(&args, "from")?.ok_or_else(|| {
                        ExecutionError::Dispatch("edit tool requires from".to_string())
                    })?;
                    if from.is_empty() {
                        return Err(ExecutionError::Dispatch(
                            "edit tool requires non-empty from".to_string(),
                        ));
                    }
                    let to = opt_str(&args, "to")?.ok_or_else(|| {
                        ExecutionError::Dispatch("edit tool requires to".to_string())
                    })?;
                    engine
                        .agent_tool_edit(path, from, to, context, options, state)
                        .await
                }
                "exec" => {
                    ensure_allowed_keys(&args, &["command", "args"])?;
                    if !options.allow_exec {
                        return Err(ExecutionError::Dispatch(
                            "agent exec is disabled; rerun with --allow-exec".to_string(),
                        ));
                    }
                    let command = opt_str(&args, "command")?.ok_or_else(|| {
                        ExecutionError::Dispatch("exec tool requires command".to_string())
                    })?;
                    let args_list = opt_str_list(&args, "args")?;
                    engine.agent_tool_exec(command, &args_list, context).await
                }
                "glob" => {
                    ensure_allowed_keys(&args, &["pattern", "path"])?;
                    let pattern = opt_str(&args, "pattern")?.ok_or_else(|| {
                        ExecutionError::Dispatch("glob tool requires pattern".to_string())
                    })?;
                    let root = opt_str(&args, "path")?;
                    engine
                        .agent_tool_glob(pattern, root, context, options)
                        .await
                }
                "grep" => {
                    ensure_allowed_keys(&args, &["pattern", "path", "include"])?;
                    let pattern = opt_str(&args, "pattern")?.ok_or_else(|| {
                        ExecutionError::Dispatch("grep tool requires pattern".to_string())
                    })?;
                    let root = opt_str(&args, "path")?;
                    let include = opt_str(&args, "include")?;
                    engine
                        .agent_tool_grep(pattern, root, include, context, options)
                        .await
                }
                "bash" => {
                    ensure_allowed_keys(&args, &["command", "timeout_secs"])?;
                    let command = opt_str(&args, "command")?.ok_or_else(|| {
                        ExecutionError::Dispatch("bash tool requires command".to_string())
                    })?;
                    let timeout_secs = opt_u64(&args, "timeout_secs")?;
                    engine.agent_tool_bash(command, timeout_secs, context).await
                }
                "apply_patch" => {
                    ensure_allowed_keys(&args, &["patch_text"])?;
                    if !options.allow_write && !options.allow_edit {
                        return Err(ExecutionError::Dispatch(
                            "apply_patch requires --allow-write or --allow-edit".to_string(),
                        ));
                    }
                    let patch_text = opt_str(&args, "patch_text")?.ok_or_else(|| {
                        ExecutionError::Dispatch("apply_patch requires patch_text".to_string())
                    })?;
                    engine
                        .agent_tool_apply_patch(patch_text, context, options, state)
                        .await
                }
                "multiedit" => {
                    ensure_allowed_keys(&args, &["path", "edits"])?;
                    if !options.allow_edit && !options.allow_write {
                        return Err(ExecutionError::Dispatch(
                            "multiedit requires --allow-edit or --allow-write".to_string(),
                        ));
                    }
                    let path = opt_str(&args, "path")?.ok_or_else(|| {
                        ExecutionError::Dispatch("multiedit requires path".to_string())
                    })?;
                    let edits_arr =
                        args.get("edits").and_then(Value::as_array).ok_or_else(|| {
                            ExecutionError::Dispatch("multiedit requires edits array".to_string())
                        })?;
                    let mut edits = Vec::with_capacity(edits_arr.len());
                    for (idx, edit) in edits_arr.iter().enumerate() {
                        let old = edit
                            .get("old_string")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(format!(
                                    "edits[{idx}].old_string required"
                                ))
                            })?
                            .to_string();
                        let new = edit
                            .get("new_string")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(format!(
                                    "edits[{idx}].new_string required"
                                ))
                            })?
                            .to_string();
                        let replace_all = edit
                            .get("replace_all")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        edits.push(MultiEditOp {
                            old_string: old,
                            new_string: new,
                            replace_all,
                        });
                    }
                    engine
                        .agent_tool_multiedit(path, &edits, context, options, state)
                        .await
                }
                "batch" => {
                    ensure_allowed_keys(&args, &["tool_calls"])?;
                    let calls = args
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            ExecutionError::Dispatch("batch requires tool_calls array".to_string())
                        })?;
                    if calls.is_empty() {
                        return Err(ExecutionError::Dispatch(
                            "batch requires at least one tool call".to_string(),
                        ));
                    }
                    let calls: Vec<_> = calls.iter().take(25).cloned().collect();
                    let discarded = args
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .map_or(0, |a| a.len().saturating_sub(25));

                    let state_mutex = Arc::new(Mutex::new(std::mem::take(state)));
                    let mut handles = Vec::with_capacity(calls.len());

                    for call in &calls {
                        let tool_name = call
                            .get("tool")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let tool_params = call
                            .get("parameters")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default()));

                        if tool_name == "batch" {
                            handles.push(Err(ExecutionError::Dispatch(
                                "batch cannot be called recursively".to_string(),
                            )));
                            continue;
                        }

                        let engine_ref = engine;
                        let context_ref = context;
                        let options_ref = options;
                        let state_ref = state_mutex.clone();

                        handles.push(Ok((
                            tool_name,
                            tool_params,
                            engine_ref,
                            context_ref,
                            options_ref,
                            state_ref,
                        )));
                    }

                    // Execute sequentially (parallel would require Send bounds we don't have)
                    let mut results = Vec::new();
                    for handle in handles {
                        match handle {
                            Err(e) => results.push(format!("error: {e}")),
                            Ok((tool_name, tool_params, eng, ctx, opts, st)) => {
                                let mut locked_state = st.lock().await;
                                let result = Self::execute(
                                    eng,
                                    &tool_name,
                                    tool_params,
                                    ctx,
                                    opts,
                                    &mut locked_state,
                                )
                                .await;
                                match result {
                                    Ok(output) => {
                                        results.push(format!("{tool_name}: ok\n{output}"));
                                    }
                                    Err(e) => results.push(format!("{tool_name}: error\n{e}")),
                                }
                            }
                        }
                    }

                    // Restore state
                    *state = Arc::try_unwrap(state_mutex)
                        .map_err(|_| ExecutionError::Executor("state lock contention".to_string()))?
                        .into_inner();

                    let successful = results.iter().filter(|r| r.contains(": ok")).count();
                    let failed = results.len() - successful;
                    let mut output = format!("batch: {successful}/{} successful", results.len());
                    if discarded > 0 {
                        output.push_str(&format!(" ({discarded} calls discarded, max 25)"));
                    }
                    if failed > 0 {
                        output.push_str(&format!(" ({failed} failed)"));
                    }
                    output.push_str("\n\n");
                    for r in &results {
                        output.push_str(r);
                        output.push_str("\n---\n");
                    }
                    Ok(output)
                }
                "question" => {
                    ensure_allowed_keys(&args, &["questions"])?;
                    let questions_arr = args
                        .get("questions")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            ExecutionError::Dispatch(
                                "question requires questions array".to_string(),
                            )
                        })?;
                    let mut questions = Vec::with_capacity(questions_arr.len());
                    for q in questions_arr {
                        let question = q
                            .get("question")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(
                                    "each question requires a 'question' field".to_string(),
                                )
                            })?
                            .to_string();
                        let options = q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(String::from)
                                    .collect()
                            })
                            .unwrap_or_default();
                        let default = q.get("default").and_then(Value::as_str).map(String::from);
                        questions.push(crate::agent_handlers_interactive::QuestionItem {
                            question,
                            options,
                            default,
                        });
                    }
                    engine.agent_tool_question(&questions).await
                }
                "plan" => {
                    ensure_allowed_keys(&args, &["title", "steps"])?;
                    let title = opt_str(&args, "title")?.ok_or_else(|| {
                        ExecutionError::Dispatch("plan requires title".to_string())
                    })?;
                    let steps_arr =
                        args.get("steps").and_then(Value::as_array).ok_or_else(|| {
                            ExecutionError::Dispatch("plan requires steps array".to_string())
                        })?;
                    let mut steps = Vec::with_capacity(steps_arr.len());
                    for s in steps_arr {
                        let description = s
                            .get("description")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(
                                    "each step requires 'description'".to_string(),
                                )
                            })?
                            .to_string();
                        let status = s
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("pending")
                            .to_string();
                        steps.push(crate::agent_handlers_interactive::PlanStep {
                            description,
                            status,
                        });
                    }
                    engine.agent_tool_plan(title, &steps).await
                }
                "websearch" => {
                    ensure_allowed_keys(&args, &["query", "num_results"])?;
                    if !context.config.allow_network {
                        return Err(ExecutionError::Dispatch(
                            "websearch requires network access".to_string(),
                        ));
                    }
                    let query = opt_str(&args, "query")?.ok_or_else(|| {
                        ExecutionError::Dispatch("websearch requires query".to_string())
                    })?;
                    let num_results = opt_u64(&args, "num_results")?;
                    engine
                        .agent_tool_websearch(query, num_results, context)
                        .await
                }
                "webfetch" => {
                    ensure_allowed_keys(&args, &["url", "format", "timeout_secs"])?;
                    let url = opt_str(&args, "url")?.ok_or_else(|| {
                        ExecutionError::Dispatch("webfetch tool requires url".to_string())
                    })?;
                    let format = opt_str(&args, "format")?;
                    let timeout_secs = opt_u64(&args, "timeout_secs")?;
                    engine
                        .agent_tool_webfetch(url, format, timeout_secs, context)
                        .await
                }
                "todowrite" => {
                    ensure_allowed_keys(&args, &["todos"])?;
                    let todos = args.get("todos").ok_or_else(|| {
                        ExecutionError::Dispatch("todowrite tool requires todos".to_string())
                    })?;
                    let list = todos.as_array().ok_or_else(|| {
                        ExecutionError::Dispatch("todos must be an array".to_string())
                    })?;

                    let mut normalized = Vec::with_capacity(list.len());
                    for (idx, item) in list.iter().enumerate() {
                        let obj = item.as_object().ok_or_else(|| {
                            ExecutionError::Dispatch(format!("todos[{idx}] must be an object"))
                        })?;
                        for key in obj.keys() {
                            if !matches!(key.as_str(), "content" | "status" | "priority") {
                                return Err(ExecutionError::Dispatch(format!(
                                    "todos[{idx}] unexpected key: {key}"
                                )));
                            }
                        }

                        let content = obj
                            .get("content")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(format!(
                                    "todos[{idx}].content must be a string"
                                ))
                            })?
                            .trim()
                            .to_string();
                        if content.is_empty() {
                            return Err(ExecutionError::Dispatch(format!(
                                "todos[{idx}].content must not be empty"
                            )));
                        }

                        let status = obj
                            .get("status")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(format!(
                                    "todos[{idx}].status must be a string"
                                ))
                            })?
                            .trim()
                            .to_ascii_lowercase();
                        if !matches!(
                            status.as_str(),
                            "pending" | "in_progress" | "completed" | "cancelled"
                        ) {
                            return Err(ExecutionError::Dispatch(format!(
                            "todos[{idx}].status must be pending|in_progress|completed|cancelled"
                        )));
                        }

                        let priority = obj
                            .get("priority")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                ExecutionError::Dispatch(format!(
                                    "todos[{idx}].priority must be a string"
                                ))
                            })?
                            .trim()
                            .to_ascii_lowercase();
                        if !matches!(priority.as_str(), "high" | "medium" | "low") {
                            return Err(ExecutionError::Dispatch(format!(
                                "todos[{idx}].priority must be high|medium|low"
                            )));
                        }

                        normalized.push(serde_json::json!({
                            "content": content,
                            "status": status,
                            "priority": priority,
                        }));
                    }

                    Ok(serde_json::json!({
                        "todos": normalized,
                    })
                    .to_string())
                }
                _ => Err(ExecutionError::Dispatch(format!(
                    "unknown tool call: {name}"
                ))),
            }
        }) // Box::pin
    }
}
