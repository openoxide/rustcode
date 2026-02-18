use serde_json::Value;

use rustcode_core::command::AgentOptions;
use rustcode_core::context::CommandContext;
use rustcode_core::error::ExecutionError;
use rustcode_llm::ToolSpec;

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
        Some(_) => Err(ExecutionError::Dispatch(format!(
            "{key} must be a string"
        ))),
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
        let mut specs = vec![
            ToolSpec {
                name: "list".to_string(),
                description: "List files and directories under a workspace-relative path.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Workspace-relative path (default: .)" }
                    },
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "read".to_string(),
                description: "Read a UTF-8 text file from the workspace.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Workspace-relative file path" }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "glob".to_string(),
                description: "Find workspace files matching a glob pattern.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Glob pattern (e.g. **/*.rs)" },
                        "path": { "type": "string", "description": "Workspace-relative search root (default: .)" }
                    },
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "grep".to_string(),
                description: "Search file contents using a regular expression.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Regex pattern" },
                        "path": { "type": "string", "description": "Workspace-relative search root (default: .)" },
                        "include": { "type": "string", "description": "Optional glob filter for file paths" }
                    },
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
            },
            ToolSpec {
                name: "todowrite".to_string(),
                description: "Update the agent's working todo list.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "content": { "type": "string" },
                                    "status": { "type": "string", "description": "pending|in_progress|completed|cancelled" },
                                    "priority": { "type": "string", "description": "high|medium|low" }
                                },
                                "required": ["content", "status", "priority"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["todos"],
                    "additionalProperties": false
                }),
            },
        ];

        if allow_network {
            specs.push(ToolSpec {
                name: "webfetch".to_string(),
                description:
                    "Fetch a URL over HTTP(S) and return its contents (optionally simplified)."
                        .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "HTTP(S) URL to fetch" },
                        "format": { "type": "string", "description": "Output format: markdown|text|html (default: markdown)" },
                        "timeout_secs": { "type": "integer", "description": "Request timeout in seconds (default: 30, max: 120)" }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
            });
        }

        if options.allow_write || options.allow_edit {
            specs.push(ToolSpec {
                name: "write".to_string(),
                description: "Write a UTF-8 text file to the workspace.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Workspace-relative file path" },
                        "contents": { "type": "string", "description": "Full file contents" }
                    },
                    "required": ["path", "contents"],
                    "additionalProperties": false
                }),
            });
        }

        if options.allow_edit {
            specs.push(ToolSpec {
                name: "edit".to_string(),
                description: "Replace a substring in a workspace file.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Workspace-relative file path" },
                        "from": { "type": "string", "description": "Exact text to replace" },
                        "to": { "type": "string", "description": "Replacement text" }
                    },
                    "required": ["path", "from", "to"],
                    "additionalProperties": false
                }),
            });
        }

        if options.allow_exec {
            specs.push(ToolSpec {
                name: "exec".to_string(),
                description: "Execute a command in the workspace.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Program name" },
                        "args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Command arguments"
                        }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            });
        }

        specs
    }

    pub async fn execute(
        engine: &Engine,
        name: &str,
        args: Value,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        match name {
            "list" => {
                ensure_allowed_keys(&args, &["path"])?;
                let path = opt_str(&args, "path")?.map(|value| value.to_string());
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
    }
}
