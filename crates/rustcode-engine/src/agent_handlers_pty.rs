// ── PTY tool handler ──────────────────────────────────────────────────
//
// Implements the `pty_exec` agent tool — like `bash` but runs the command
// inside a real pseudo-terminal so `isatty()` returns true.
//
// Use when a command changes behaviour based on whether it is attached to
// a terminal (REPLs, colour output, test runners like `npm test`, etc.).

use std::time::Duration;

use rustcode_io::IoError;

use super::{CommandContext, Engine, ExecutionError};
use crate::agent_handlers_bash::{detect_shell, shell_exec_args, truncate_output};

/// Maximum output bytes before truncation (512 KB — larger than bash default).
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// Default command timeout (2 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum allowed timeout (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

impl Engine {
    /// Execute `command` inside a PTY with optional timeout.
    ///
    /// The command is wrapped in the user's shell (same as the `bash` tool)
    /// but runs in a real pseudo-terminal so programs that inspect `isatty()`
    /// receive `true`.
    pub(crate) async fn agent_tool_pty_exec(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let timeout_secs = timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let shell = detect_shell();
        let shell_flag = shell_exec_args(&shell);

        let pty_result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            self.process.run_pty(
                &shell,
                &[shell_flag.to_string(), command.to_string()],
                &context.config.workspace_root,
                context.cancellation.clone(),
            ),
        )
        .await
        .map_err(|_| {
            ExecutionError::Executor(format!(
                "pty command timed out after {timeout_secs}s: {command}"
            ))
        })?
        .map_err(|err| match err {
            IoError::Cancelled => ExecutionError::Cancelled,
            _ => ExecutionError::Executor(err.to_string()),
        })?;

        let mut rendered = String::new();
        rendered.push_str("exit_code=");
        rendered.push_str(&pty_result.exit_code.to_string());
        rendered.push('\n');

        if !pty_result.output.is_empty() {
            rendered.push_str("output:\n");
            rendered.push_str(&pty_result.output);
            if !pty_result.output.ends_with('\n') {
                rendered.push('\n');
            }
        }

        // Truncate if output is too large
        if rendered.len() > MAX_OUTPUT_BYTES {
            let truncated = truncate_output(&rendered, MAX_OUTPUT_BYTES);
            return if pty_result.exit_code == 0 {
                Ok(truncated)
            } else {
                Err(ExecutionError::Executor(truncated))
            };
        }

        if pty_result.exit_code == 0 {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustcode_core::config::ResolvedConfig;
    use rustcode_io::LocalIo;
    use tokio_util::sync::CancellationToken;

    use crate::{Engine, WorkspacePermissionPolicy};

    fn make_engine() -> Engine {
        let io = Arc::new(LocalIo);
        Engine::new(
            Arc::new(rustcode_llm::NullLlmClient),
            io.clone(),
            io,
            Arc::new(WorkspacePermissionPolicy),
            rustcode_plugins::PluginRegistry::default(),
            None,
            None,
        )
    }

    fn make_context() -> rustcode_core::context::CommandContext {
        let config = ResolvedConfig {
            workspace_root: std::env::temp_dir(),
            model: "null".to_string(),
            ..ResolvedConfig::default()
        };
        rustcode_core::context::CommandContext {
            session: rustcode_core::context::SessionMeta {
                session_id: "test".to_string(),
                request_id: "req-test".to_string(),
                started_at: std::time::SystemTime::now(),
            },
            config: Arc::new(config),
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn pty_exec_echo_succeeds() {
        let engine = make_engine();
        let ctx = make_context();
        let result = engine
            .agent_tool_pty_exec("echo hello_pty", None, &ctx)
            .await;
        let out = result.expect("pty_exec should succeed");
        assert!(out.contains("exit_code=0"), "out={out}");
        assert!(out.contains("hello_pty"), "out={out}");
    }

    #[tokio::test]
    async fn pty_exec_nonzero_exit_is_error() {
        let engine = make_engine();
        let ctx = make_context();
        let result = engine.agent_tool_pty_exec("exit 7", None, &ctx).await;
        assert!(result.is_err(), "expected error for non-zero exit");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exit_code=7") || msg.contains("7"),
            "msg={msg}"
        );
    }
}
