use rustcode_core::command::AgentOptions;
use rustcode_llm::ToolSpec;

/// Build the list of tool specifications available to the agent.
///
/// This is separated from the dispatch logic in `agent_tools.rs` to keep
/// both files under the 600 LOC limit.
pub fn tool_specs(options: &AgentOptions, allow_network: bool) -> Vec<ToolSpec> {
    let mut specs = vec![
        ToolSpec {
            name: "list".to_string(),
            description: "List files and directories under a workspace-relative path."
                .to_string(),
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

    // bash tool — always available, primary shell execution tool
    specs.push(ToolSpec {
        name: "bash".to_string(),
        description: "Execute a shell command in the workspace. Commands run in the user's default shell with configurable timeout.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default: 120, max: 600)" }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    });

    // apply_patch + multiedit — available if write/edit allowed
    if options.allow_write || options.allow_edit {
        specs.push(ToolSpec {
            name: "apply_patch".to_string(),
            description: "Apply a unified diff patch to one or more workspace files.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch_text": { "type": "string", "description": "The full unified diff/patch text" }
                },
                "required": ["patch_text"],
                "additionalProperties": false
            }),
        });

        specs.push(ToolSpec {
            name: "multiedit".to_string(),
            description: "Apply multiple sequential edits to a single file. Each edit replaces old_string with new_string.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string", "description": "Text to replace" },
                                "new_string": { "type": "string", "description": "Replacement text" },
                                "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)" }
                            },
                            "required": ["old_string", "new_string"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }),
        });
    }

    // batch tool — run multiple tools in parallel
    specs.push(ToolSpec {
        name: "batch".to_string(),
        description: "Execute multiple tool calls in parallel for better performance. Maximum 25 calls per batch.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "tool_calls": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string", "description": "The tool name" },
                            "parameters": { "type": "object", "description": "Parameters for the tool" }
                        },
                        "required": ["tool", "parameters"]
                    },
                    "description": "Array of tool calls to execute in parallel"
                }
            },
            "required": ["tool_calls"],
            "additionalProperties": false
        }),
    });

    // question tool — ask user questions during agent loop
    specs.push(ToolSpec {
        name: "question".to_string(),
        description: "Ask the user one or more questions. Use when you need clarification or input before proceeding.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The question to ask" },
                            "options": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Optional list of choices"
                            },
                            "default": { "type": "string", "description": "Optional default answer" }
                        },
                        "required": ["question"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    });

    // plan tool — structured task planning
    specs.push(ToolSpec {
        name: "plan".to_string(),
        description: "Create or update a structured plan with steps and status tracking.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Plan title" },
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": { "type": "string", "description": "Step description" },
                            "status": { "type": "string", "description": "pending|in_progress|completed|blocked" }
                        },
                        "required": ["description", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["title", "steps"],
            "additionalProperties": false
        }),
    });

    if allow_network {
        // websearch tool — search the web
        specs.push(ToolSpec {
            name: "websearch".to_string(),
            description: "Search the web for information. Returns summarized search results.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "num_results": { "type": "integer", "description": "Number of results (default: 5, max: 10)" }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        });

        // webfetch tool — also under allow_network guard
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
