use globset::Glob;
use serde_json::Value;

use rustcode_core::error::ExecutionError;
use rustcode_core::permissions::PermissionAction;

/// Determine whether a tool is mutating (requires approval).
pub(crate) fn is_mutating_tool(name: &str) -> bool {
    matches!(name, "write" | "edit" | "exec") || name.starts_with("mcp:")
}

/// Determine whether a tool can safely execute in parallel with other tools.
pub(crate) fn is_parallel_safe_tool(name: &str) -> bool {
    matches!(name, "list" | "glob" | "grep" | "webfetch" | "todowrite")
}

/// Resolve a permission action for a tool call against the permission rule stack.
pub(crate) fn resolve_permission_action(
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

/// Extract structured approval fields from a tool call for display to the user.
pub(crate) fn approval_fields(tool: &str, args: &Value) -> (String, String, String) {
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

/// Determine the glob match targets for a tool call permission check.
pub(crate) fn approval_match_targets(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        _ if tool.starts_with("mcp:") => vec![tool.to_string()],
        "exec" => exec_match_targets(args),
        "write" | "edit" => vec![args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        _ => vec!["*".to_string()],
    }
}

fn exec_match_targets(args: &Value) -> Vec<String> {
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

/// Truncate a string to fit within `max_bytes` on a UTF-8 char boundary.
pub(crate) fn truncate_utf8_bytes(input: &str, max_bytes: usize) -> String {
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

/// Convert a tool result into a JSON payload, truncating if necessary.
pub(crate) fn tool_payload_json(ok: bool, output: String, max_bytes: usize) -> String {
    let mut truncated = false;
    let mut candidate_output = output;

    for _ in 0..8 {
        let payload = serde_json::json!({
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

    serde_json::json!({
        "ok": ok,
        "truncated": true,
        "output": "[tool output truncated]".to_string(),
    })
    .to_string()
}

/// Convert stored messages to chat messages for the LLM request.
pub(crate) fn stored_messages_to_chat(
    history: Vec<rustcode_core::session::StoredMessage>,
) -> Vec<rustcode_llm::ChatMessage> {
    use rustcode_core::session::MessageRole;
    use rustcode_llm::{ChatMessage, ChatRole, ToolCall};

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
