use serde_json::Value;

use rustcode_core::command::AgentOptions;
use rustcode_core::context::CommandContext;
use rustcode_core::error::ExecutionError;
use rustcode_llm::ToolSpec;

use crate::{AgentState, Engine};

pub struct AgentToolRegistry;

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
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                engine.agent_tool_list(path, context, options).await
            }
            "read" => {
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("read tool requires path".to_string())
                })?;
                engine.agent_tool_read(path, context, options, state).await
            }
            "write" => {
                if !options.allow_write && !options.allow_edit {
                    return Err(ExecutionError::Dispatch(
                        "agent write is disabled; rerun with --allow-write".to_string(),
                    ));
                }
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("write tool requires path".to_string())
                })?;
                let contents = args
                    .get("contents")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ExecutionError::Dispatch("write tool requires contents".to_string())
                    })?;
                engine
                    .agent_tool_write(path, contents, context, options, state)
                    .await
            }
            "edit" => {
                if !options.allow_edit {
                    return Err(ExecutionError::Dispatch(
                        "agent edit is disabled; rerun with --allow-edit".to_string(),
                    ));
                }
                let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("edit tool requires path".to_string())
                })?;
                let from = args.get("from").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("edit tool requires from".to_string())
                })?;
                if from.is_empty() {
                    return Err(ExecutionError::Dispatch(
                        "edit tool requires non-empty from".to_string(),
                    ));
                }
                let to = args.get("to").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("edit tool requires to".to_string())
                })?;
                engine
                    .agent_tool_edit(path, from, to, context, options, state)
                    .await
            }
            "exec" => {
                if !options.allow_exec {
                    return Err(ExecutionError::Dispatch(
                        "agent exec is disabled; rerun with --allow-exec".to_string(),
                    ));
                }
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ExecutionError::Dispatch("exec tool requires command".to_string())
                    })?;
                let args_list = args
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                engine.agent_tool_exec(command, &args_list, context).await
            }
            "glob" => {
                let pattern = args.get("pattern").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("glob tool requires pattern".to_string())
                })?;
                let root = args.get("path").and_then(Value::as_str);
                engine
                    .agent_tool_glob(pattern, root, context, options)
                    .await
            }
            "grep" => {
                let pattern = args.get("pattern").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("grep tool requires pattern".to_string())
                })?;
                let root = args.get("path").and_then(Value::as_str);
                let include = args.get("include").and_then(Value::as_str);
                engine
                    .agent_tool_grep(pattern, root, include, context, options)
                    .await
            }
            "webfetch" => {
                let url = args.get("url").and_then(Value::as_str).ok_or_else(|| {
                    ExecutionError::Dispatch("webfetch tool requires url".to_string())
                })?;
                let format = args.get("format").and_then(Value::as_str);
                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(Value::as_u64);
                engine
                    .agent_tool_webfetch(url, format, timeout_secs, context)
                    .await
            }
            _ => Err(ExecutionError::Dispatch(format!(
                "unknown tool call: {name}"
            ))),
        }
    }
}
