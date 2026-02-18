use anyhow::Result;
use async_trait::async_trait;
use rustcode_core::ports::ToolApprover;
use rustcode_core::tool_approval::ToolApprovalRequest;
use rustcode_core::error::ExecutionError;
use crate::utils::{is_interactive_terminal, write_stdout_line, write_stdout_raw};

pub struct StdioToolApprover;

#[async_trait]
impl ToolApprover for StdioToolApprover {
    async fn approve(
        &self,
        request: ToolApprovalRequest,
    ) -> Result<bool, ExecutionError> {
        if !is_interactive_terminal() {
            return Ok(false);
        }

        if let Err(err) = write_stdout_line(&format!("Tool call: {} with args: {}", request.tool, request.arguments)) {
             return Err(ExecutionError::Executor(err.to_string()));
        }
        if let Err(err) = write_stdout_raw("Approve? [y/N] ") {
             return Err(ExecutionError::Executor(err.to_string()));
        }
        
        let mut input = String::new();
        if let Err(err) = std::io::stdin().read_line(&mut input) {
             return Err(ExecutionError::Executor(err.to_string()));
        }
        let input = input.trim().to_lowercase();
        
        if input == "y" || input == "yes" {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
