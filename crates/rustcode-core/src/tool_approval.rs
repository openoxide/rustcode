use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolApprovalRequest {
    pub tool: String,
    pub permission: String,
    pub pattern: String,
    pub arguments: Value,
    pub reason: String,
}
