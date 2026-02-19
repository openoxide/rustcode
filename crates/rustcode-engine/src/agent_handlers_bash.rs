use std::time::Duration;

use super::{CommandContext, Engine, ExecutionError, IoError};

/// Maximum output length before truncation (128 KB).
const MAX_OUTPUT_BYTES: usize = 128 * 1024;

/// Default timeout for bash commands (2 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum allowed timeout (10 minutes).
const MAX_TIMEOUT_SECS: u64 = 600;

impl Engine {
    /// Execute a shell command with timeout and output truncation.
    ///
    /// Unlike the raw `exec` tool, `bash` wraps commands in the user's shell
    /// (or `/bin/sh` as fallback), supports configurable timeouts, and
    /// truncates excessively long output to keep context windows manageable.
    pub(crate) async fn agent_tool_bash(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let timeout_secs = timeout_secs
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let shell = detect_shell();
        let shell_args = shell_exec_args(&shell);

        let output = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            self.process.run_capture(
                &shell,
                &[shell_args.to_string(), command.to_string()],
                &context.config.workspace_root,
                context.cancellation.clone(),
            ),
        )
        .await
        .map_err(|_| {
            ExecutionError::Executor(format!(
                "command timed out after {timeout_secs}s: {command}"
            ))
        })?
        .map_err(|err| match err {
            IoError::Cancelled => ExecutionError::Cancelled,
            _ => ExecutionError::Executor(err.to_string()),
        })?;

        let mut rendered = String::new();
        rendered.push_str("exit_code=");
        rendered.push_str(&output.code.to_string());
        rendered.push('\n');

        if !output.stdout.is_empty() {
            rendered.push_str("stdout:\n");
            rendered.push_str(&output.stdout);
            if !output.stdout.ends_with('\n') {
                rendered.push('\n');
            }
        }
        if !output.stderr.is_empty() {
            rendered.push_str("stderr:\n");
            rendered.push_str(&output.stderr);
            if !output.stderr.ends_with('\n') {
                rendered.push('\n');
            }
        }

        // Truncate if output is too large
        if rendered.len() > MAX_OUTPUT_BYTES {
            let truncated = truncate_output(&rendered, MAX_OUTPUT_BYTES);
            return if output.code == 0 {
                Ok(truncated)
            } else {
                Err(ExecutionError::Executor(truncated))
            };
        }

        if output.code == 0 {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }
}

/// Detect the user's preferred shell.
pub(crate) fn detect_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }
    // Fallback
    if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

/// Return the shell flag for executing a command string.
pub(crate) fn shell_exec_args(shell: &str) -> &'static str {
    if shell.ends_with("cmd.exe") || shell.ends_with("cmd") {
        "/C"
    } else {
        "-c"
    }
}

/// Truncate output preserving head and tail with a marker in between.
pub(crate) fn truncate_output(output: &str, max_bytes: usize) -> String {
    let keep = max_bytes / 2;
    let head = &output[..keep];
    let tail = &output[output.len() - keep..];
    let omitted = output.len() - max_bytes;
    format!("{head}\n\n[rustcode: output truncated — {omitted} bytes omitted]\n\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_returns_nonempty() {
        let shell = detect_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    fn shell_exec_args_sh() {
        assert_eq!(shell_exec_args("/bin/bash"), "-c");
        assert_eq!(shell_exec_args("/bin/zsh"), "-c");
    }

    #[test]
    fn truncate_output_large() {
        let data = "x".repeat(200);
        let truncated = truncate_output(&data, 100);
        assert!(truncated.contains("[rustcode: output truncated"));
        assert!(truncated.len() < 200 + 100); // head + marker + tail
    }

    #[test]
    fn truncate_output_preserves_head_and_tail() {
        let data = format!("{}MIDDLE{}", "A".repeat(100), "Z".repeat(100));
        let truncated = truncate_output(&data, 100);
        assert!(truncated.starts_with("AAAA"));
        assert!(truncated.ends_with("ZZZZ"));
    }
}
