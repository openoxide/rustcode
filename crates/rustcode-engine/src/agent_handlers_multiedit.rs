use super::{AgentOptions, AgentState, CommandContext, Engine, ExecutionError, PathOperation};
use crate::agent_handlers_fs::diff_output;

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
        // Auto-snapshot before the first mutation this session
        self.auto_snapshot_before_mutation(state, context).await;
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
        let original = content.clone();

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

        let base_msg = format!(
            "multiedit applied ({} edits, {} replacements) to {}",
            edits.len(),
            total_replacements,
            resolved.display()
        );
        let diff = diff_output(&original, &content, 40);
        if diff.is_empty() {
            Ok(base_msg)
        } else {
            Ok(format!("{base_msg}\n{diff}"))
        }
    }
}

/// A single edit operation within a multiedit call.
pub struct MultiEditOp {
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiedit_op_basic() {
        let op = MultiEditOp {
            old_string: "foo".to_string(),
            new_string: "bar".to_string(),
            replace_all: false,
        };
        assert_eq!(op.old_string, "foo");
        assert_eq!(op.new_string, "bar");
        assert!(!op.replace_all);
    }

    #[test]
    fn multiedit_op_replace_all() {
        let op = MultiEditOp {
            old_string: "hello".to_string(),
            new_string: "world".to_string(),
            replace_all: true,
        };
        assert!(op.replace_all);
    }

    #[test]
    fn multiedit_logic_single_replace() {
        // Simulate the core replace logic from agent_tool_multiedit
        let content = "hello world hello";
        let old = "hello";
        let new = "hi";
        // First occurrence only
        if let Some(pos) = content.find(old) {
            let result = format!("{}{}{}", &content[..pos], new, &content[pos + old.len()..]);
            assert_eq!(result, "hi world hello");
        }
    }

    #[test]
    fn multiedit_logic_replace_all() {
        let content = "hello world hello";
        let result = content.replace("hello", "hi");
        assert_eq!(result, "hi world hi");
    }

    #[test]
    fn multiedit_logic_sequential_edits() {
        // Verify sequential edit application
        let mut content = "aaa bbb ccc".to_string();
        content = content.replace("aaa", "xxx");
        content = content.replace("ccc", "zzz");
        assert_eq!(content, "xxx bbb zzz");
    }
}
