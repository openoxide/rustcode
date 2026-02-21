//! Tool permission checking and MCP tool execution.

use serde_json::Value;

use rustcode_core::context::CommandContext;
use rustcode_core::error::ExecutionError;
use rustcode_core::permissions::PermissionAction;
use rustcode_core::ToolApprovalRequest;

use crate::agent_util::{approval_fields, approval_match_targets, resolve_permission_action};
use crate::Engine;

impl Engine {
    pub(crate) async fn check_tool_permission(
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

    pub(crate) async fn execute_mcp_tool(
        &self,
        name: &str,
        args: Value,
    ) -> Result<String, ExecutionError> {
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
