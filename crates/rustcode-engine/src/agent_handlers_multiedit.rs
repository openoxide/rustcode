use super::{AgentOptions, AgentState, CommandContext, Engine, ExecutionError, PathOperation};

impl Engine {
    /// Apply multiple sequential edits to a single file.
    ///
    /// Each edit specifies `old_string` and `new_string`. Edits are applied
    /// in order, so later edits see the result of earlier ones.
    /// If `replace_all` is true for an edit, all occurrences are replaced.
    pub(crate) async fn agent_tool_multiedit(
        &self,
        path: &str,
        edits: &[MultiEditOp],
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        if edits.is_empty() {
            return Err(ExecutionError::Dispatch(
                "multiedit requires at least one edit operation".to_string(),
            ));
        }

        let resolved = self.resolve_workspace_path(context, path, PathOperation::Edit)?;
        if !state.read_paths.contains(&resolved) {
            return Err(ExecutionError::Dispatch(
                "multiedit requires reading the target file first".to_string(),
            ));
        }

        let mut content = self
            .fs
            .read_to_string_limited(&resolved, options.max_read_bytes)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        if content.contains("[rustcode:truncated]") {
            return Err(ExecutionError::Dispatch(
                "refusing to edit a truncated read; increase --max-read-bytes".to_string(),
            ));
        }

        let mut total_replacements = 0usize;
        for (idx, edit) in edits.iter().enumerate() {
            if edit.old_string.is_empty() {
                return Err(ExecutionError::Dispatch(format!(
                    "edits[{idx}].old_string must not be empty"
                )));
            }
            if edit.old_string == edit.new_string {
                return Err(ExecutionError::Dispatch(format!(
                    "edits[{idx}].old_string must differ from new_string"
                )));
            }

            let count = content.matches(&edit.old_string).count();
            if count == 0 {
                return Err(ExecutionError::Dispatch(format!(
                    "edits[{idx}].old_string not found in file"
                )));
            }
            if count > 1 && !edit.replace_all {
                return Err(ExecutionError::Dispatch(format!(
                    "edits[{idx}].old_string found {count} times; set replace_all=true or make the match unique"
                )));
            }

            if edit.replace_all {
                content = content.replace(&edit.old_string, &edit.new_string);
                total_replacements += count;
            } else {
                // Replace only the first occurrence
                if let Some(pos) = content.find(&edit.old_string) {
                    content = format!(
                        "{}{}{}",
                        &content[..pos],
                        edit.new_string,
                        &content[pos + edit.old_string.len()..]
                    );
                    total_replacements += 1;
                }
            }
        }

        self.fs
            .write_string(&resolved, &content)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        Ok(format!(
            "multiedit applied ({} edits, {} replacements) to {}",
            edits.len(),
            total_replacements,
            resolved.display()
        ))
    }
}

/// A single edit operation within a multiedit call.
pub struct MultiEditOp {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}
