use globset::Glob;
use serde_json::Value;

use rustcode_core::error::ExecutionError;
use rustcode_core::permissions::PermissionAction;

/// Determine whether a tool call requires user approval.
///
/// Allowlisted (no approval): lsp, question, plan, task, codesearch.
/// `list`, `read`, `glob`, and `grep` are allowlisted only when scoped to the
/// current dir (`path` omitted/`.` for list|glob|grep, and a direct file in `.`
/// for read).
/// All other tools require approval.
pub(crate) fn is_mutating_tool(name: &str, args: &Value) -> bool {
    if name.starts_with("mcp:") {
        return true;
    }
    if matches!(name, "lsp" | "question" | "plan" | "task" | "codesearch") {
        return false;
    }
    if matches!(name, "list" | "glob" | "grep") {
        return !tool_targets_current_dir(args);
    }
    if name == "read" {
        return !read_targets_current_dir(args);
    }
    true
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
        "bash" | "pty_exec" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                command.to_string(),
                format!("agent requests permission to execute shell command {command}"),
            )
        }
        "multiedit" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to apply multiple edits to {path}"),
            )
        }
        "apply_patch" => {
            let patch = args.get("patch_text").and_then(Value::as_str).unwrap_or("");
            let target = extract_first_patch_target(patch).unwrap_or_else(|| "<patch>".to_string());
            (
                target.clone(),
                format!("agent requests permission to apply patch ({target})"),
            )
        }
        "webfetch" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                url.to_string(),
                format!("agent requests permission to fetch {url}"),
            )
        }
        "websearch" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                query.to_string(),
                format!("agent requests permission to web-search for {query}"),
            )
        }
        "glob" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
            let pattern = args.get("pattern").and_then(Value::as_str).unwrap_or("*");
            (
                path.to_string(),
                format!("agent requests permission to glob in {path} with pattern {pattern}"),
            )
        }
        "worktree_create" => {
            let branch = args
                .get("branch")
                .and_then(Value::as_str)
                .or_else(|| args.get("name").and_then(Value::as_str))
                .unwrap_or("<auto>");
            (
                branch.to_string(),
                format!("agent requests permission to create worktree {branch}"),
            )
        }
        "worktree_remove" | "worktree_reset" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                path.to_string(),
                format!("agent requests permission to run {tool} on {path}"),
            )
        }
        "snapshot_restore" => {
            let hash = args
                .get("hash")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            (
                hash.to_string(),
                format!("agent requests permission to restore snapshot {hash}"),
            )
        }
        _ => (
            tool.to_string(),
            format!("agent requests permission to call tool {tool}"),
        ),
    };

    (permission, pattern, reason)
}

/// Determine the glob match targets for a tool call permission check.
pub(crate) fn approval_match_targets(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        _ if tool.starts_with("mcp:") => vec![tool.to_string()],
        "exec" => exec_match_targets(args),
        "bash" | "pty_exec" => args
            .get("command")
            .and_then(Value::as_str)
            .map(|command| vec![command.to_string()])
            .unwrap_or_else(|| vec![String::new()]),
        "write" | "edit" | "multiedit" | "worktree_remove" | "worktree_reset" => vec![args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        "apply_patch" => args
            .get("patch_text")
            .and_then(Value::as_str)
            .and_then(extract_first_patch_target)
            .map(|target| vec![target])
            .unwrap_or_else(|| vec!["*".to_string()]),
        "glob" | "grep" | "list" | "read" => vec![args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_string()],
        "webfetch" => vec![args
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        "websearch" | "codesearch" => vec![args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        "worktree_create" => vec![args
            .get("branch")
            .and_then(Value::as_str)
            .or_else(|| args.get("name").and_then(Value::as_str))
            .unwrap_or("")
            .to_string()],
        "snapshot_restore" => vec![args
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()],
        _ => vec![tool.to_string()],
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

fn tool_targets_current_dir(args: &Value) -> bool {
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let normalized = normalize_path(path);
    let trimmed = normalized.trim_end_matches('/');
    trimmed.is_empty() || trimmed == "."
}

fn read_targets_current_dir(args: &Value) -> bool {
    let Some(path) = args.get("path").and_then(Value::as_str) else {
        return false;
    };
    let normalized = normalize_path(path);
    if normalized.is_empty() || normalized == "." {
        return false;
    }
    !normalized.contains('/')
}

fn normalize_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized
}

fn extract_first_patch_target(patch_text: &str) -> Option<String> {
    for line in patch_text.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            let trimmed = path.trim().trim_start_matches("b/");
            if !trimmed.is_empty() && trimmed != "/dev/null" {
                return Some(trimmed.to_string());
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_tools_do_not_require_approval() {
        let args = serde_json::json!({});
        for tool in ["lsp", "question", "plan", "task", "codesearch"] {
            assert!(!is_mutating_tool(tool, &args), "tool={tool}");
        }
    }

    #[test]
    fn glob_requires_approval_only_outside_current_dir() {
        assert!(!is_mutating_tool(
            "glob",
            &serde_json::json!({"pattern":"*.rs"})
        ));
        assert!(!is_mutating_tool(
            "glob",
            &serde_json::json!({"pattern":"*.rs","path":"."})
        ));
        assert!(!is_mutating_tool(
            "glob",
            &serde_json::json!({"pattern":"*.rs","path":"./"})
        ));
        assert!(is_mutating_tool(
            "glob",
            &serde_json::json!({"pattern":"*.rs","path":"src"})
        ));
    }

    #[test]
    fn list_and_grep_require_approval_outside_current_dir() {
        assert!(!is_mutating_tool("list", &serde_json::json!({})));
        assert!(!is_mutating_tool(
            "grep",
            &serde_json::json!({"pattern":"todo"})
        ));
        assert!(is_mutating_tool("list", &serde_json::json!({"path":"src"})));
        assert!(is_mutating_tool(
            "grep",
            &serde_json::json!({"pattern":"todo","path":"src"})
        ));
    }

    #[test]
    fn read_requires_approval_for_nested_paths() {
        assert!(!is_mutating_tool(
            "read",
            &serde_json::json!({"path":"main.rs"})
        ));
        assert!(!is_mutating_tool(
            "read",
            &serde_json::json!({"path":"./main.rs"})
        ));
        assert!(is_mutating_tool(
            "read",
            &serde_json::json!({"path":"src/main.rs"})
        ));
    }
}
