mod dispatch;

use serde_json::Value;

use rustcode_core::command::AgentOptions;
use rustcode_core::context::CommandContext;
use rustcode_core::error::ExecutionError;
use rustcode_llm::ToolSpec;

use crate::{AgentState, Engine};

pub struct AgentToolRegistry;

pub(crate) fn ensure_allowed_keys(args: &Value, allowed: &[&str]) -> Result<(), ExecutionError> {
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

pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ExecutionError> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ExecutionError::Dispatch(format!("{key} must be a string"))),
    }
}

pub(crate) fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, ExecutionError> {
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

pub(crate) fn opt_u32(args: &Value, key: &str) -> Result<Option<u32>, ExecutionError> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Number(num)) => num.as_u64().map(|n| Some(n as u32)).ok_or_else(|| {
            ExecutionError::Dispatch(format!("{key} must be a non-negative integer"))
        }),
        Some(_) => Err(ExecutionError::Dispatch(format!(
            "{key} must be an integer"
        ))),
    }
}

pub(crate) fn opt_str_list(args: &Value, key: &str) -> Result<Vec<String>, ExecutionError> {
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
    pub fn tool_specs(options: &AgentOptions, allow_network: bool, has_lsp: bool) -> Vec<ToolSpec> {
        super::agent_tool_specs::tool_specs(options, allow_network, has_lsp)
    }

    pub fn execute<'a>(
        engine: &'a Engine,
        name: &'a str,
        args: Value,
        context: &'a CommandContext,
        options: &'a AgentOptions,
        state: &'a mut AgentState,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, ExecutionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            dispatch::execute_tool(engine, name, args, context, options, state).await
        })
    }
}
