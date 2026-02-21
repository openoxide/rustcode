use super::{
    path_utils, AgentOptions, AgentState, CommandContext, Engine, ExecutionError, Glob, IoError,
    PathOperation, Regex,
};

/// Compute a compact diff between `old` and `new`.
///
/// Returns a string with a summary line (`+N -M`) followed by `@@diff` marker
/// and coloured diff lines.  Returns an empty string when `old == new` or
/// both are empty.
///
/// At most `max_diff_lines` `+`/`-` lines are included; excess are summarised
/// with a `...[N more]` note.
pub(crate) fn diff_output(old: &str, new: &str, max_diff_lines: usize) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    if added == 0 && removed == 0 {
        return String::new();
    }

    // Render grouped hunks with 2 lines of context above/below each change.
    let mut lines: Vec<String> = Vec::new();
    let mut emitted_changes = 0usize;
    let mut omitted_changes = 0usize;
    for group in diff.grouped_ops(2) {
        if group.is_empty() {
            continue;
        }
        if emitted_changes >= max_diff_lines {
            omitted_changes += changed_lines_in_group(&diff, &group);
            continue;
        }

        let old_start = group[0].old_range().start + 1;
        let old_end = group[group.len() - 1].old_range().end;
        let new_start = group[0].new_range().start + 1;
        let new_end = group[group.len() - 1].new_range().end;
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start,
            old_end.saturating_sub(group[0].old_range().start),
            new_start,
            new_end.saturating_sub(group[0].new_range().start)
        ));

        for op in group {
            for change in diff.iter_changes(&op) {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Insert => {
                        if emitted_changes < max_diff_lines {
                            lines.push(format!("+{line}"));
                            emitted_changes += 1;
                        } else {
                            omitted_changes += 1;
                        }
                    }
                    ChangeTag::Delete => {
                        if emitted_changes < max_diff_lines {
                            lines.push(format!("-{line}"));
                            emitted_changes += 1;
                        } else {
                            omitted_changes += 1;
                        }
                    }
                    ChangeTag::Equal => {
                        if emitted_changes < max_diff_lines {
                            lines.push(format!(" {line}"));
                        }
                    }
                }
            }
        }
    }

    if omitted_changes > 0 {
        lines.push(format!("...[{omitted_changes} more diff lines]"));
    }

    format!("+{added} -{removed}\n@@diff\n{}", lines.join("\n"))
}

fn changed_lines_in_group(
    diff: &similar::TextDiff<'_, '_, '_, str>,
    group: &[similar::DiffOp],
) -> usize {
    let mut count = 0usize;
    for op in group {
        for change in diff.iter_changes(op) {
            if !matches!(change.tag(), similar::ChangeTag::Equal) {
                count += 1;
            }
        }
    }
    count
}

impl Engine {
    pub(crate) async fn agent_tool_exec(
        &self,
        command: &str,
        args: &[String],
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let output = self
            .process
            .run_capture(
                command,
                args,
                &context.config.workspace_root,
                context.cancellation.clone(),
            )
            .await
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

        if output.code == 0 {
            Ok(rendered)
        } else {
            Err(ExecutionError::Executor(rendered))
        }
    }

    pub(crate) async fn agent_tool_list(
        &self,
        path: Option<String>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let target = path.unwrap_or_else(|| ".".to_string());
        let resolved = self.resolve_workspace_path(context, &target, PathOperation::List)?;
        let entries = self
            .fs
            .list_dir_limited(&resolved, options.max_list_entries)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        let workspace_root = path_utils::absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        for entry in entries {
            let relative = entry
                .strip_prefix(&workspace_root)
                .unwrap_or(&entry)
                .display()
                .to_string();
            rendered.push_str(&relative);
            rendered.push('\n');
        }
        Ok(rendered)
    }

    pub(crate) async fn agent_tool_read(
        &self,
        path: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Read)?;
        state.read_paths.insert(resolved.clone());
        self.fs
            .read_to_string_limited(&resolved, options.max_read_bytes)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))
    }

    pub(crate) async fn agent_tool_write(
        &self,
        path: &str,
        contents: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        // Auto-snapshot before the first mutation this session
        self.auto_snapshot_before_mutation(state, context).await;
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Write)?;
        let file_exists = self
            .fs
            .exists(&resolved)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        if !state.read_paths.contains(&resolved) && file_exists {
            return Err(ExecutionError::Dispatch(
                "write requires reading the target file first (refusing to overwrite unread file)"
                    .to_string(),
            ));
        }
        if contents.len() > options.max_write_bytes {
            return Err(ExecutionError::Dispatch(format!(
                "write contents exceeds max-write-bytes limit ({} > {})",
                contents.len(),
                options.max_write_bytes
            )));
        }
        // Capture old content for diff (only if file was previously read)
        let old_content = if state.read_paths.contains(&resolved) && file_exists {
            self.fs
                .read_to_string_limited(&resolved, options.max_read_bytes)
                .await
                .ok()
        } else {
            None
        };
        self.fs
            .write_string(&resolved, contents)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        let base_msg = format!("wrote {} bytes to {}", contents.len(), resolved.display());
        if let Some(old) = old_content {
            // Existing file was modified — show changed lines.
            let diff = diff_output(&old, contents, 40);
            if !diff.is_empty() {
                return Ok(format!("{base_msg}\n{diff}"));
            }
        } else if !file_exists {
            // New file — show content as all-addition diff (GitHub green in TUI).
            let diff = diff_output("", contents, 40);
            if !diff.is_empty() {
                return Ok(format!("{base_msg}\n{diff}"));
            }
        }
        Ok(base_msg)
    }

    pub(crate) async fn agent_tool_edit(
        &self,
        path: &str,
        from: &str,
        to: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        // Auto-snapshot before the first mutation this session
        self.auto_snapshot_before_mutation(state, context).await;
        let resolved = self.resolve_workspace_path(context, path, PathOperation::Edit)?;
        if !state.read_paths.contains(&resolved) {
            return Err(ExecutionError::Dispatch(
                "edit requires reading the target file first".to_string(),
            ));
        }
        let original = self
            .fs
            .read_to_string_limited(&resolved, options.max_read_bytes)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
        if original.contains("[rustcode:truncated]") {
            return Err(ExecutionError::Dispatch(
                "refusing to edit a truncated read; increase --max-read-bytes".to_string(),
            ));
        }

        let replacements = original.matches(from).count();
        let updated = original.replace(from, to);
        self.fs
            .write_string(&resolved, &updated)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let base_msg = format!(
            "edit applied ({replacements} replacements) to {}",
            resolved.display()
        );
        let diff = diff_output(&original, &updated, 40);
        if diff.is_empty() {
            Ok(base_msg)
        } else {
            Ok(format!("{base_msg}\n{diff}"))
        }
    }

    pub(crate) async fn agent_tool_glob(
        &self,
        pattern: &str,
        root: Option<&str>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let root = root.unwrap_or(".");
        let resolved_root = self.resolve_workspace_path(context, root, PathOperation::List)?;
        let matcher = Glob::new(pattern)
            .map_err(|err| ExecutionError::Dispatch(format!("invalid glob pattern: {err}")))?
            .compile_matcher();
        let workspace_root = path_utils::absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let candidates = self
            .fs
            .walk_dir_limited(&resolved_root, options.max_list_entries)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        let mut count = 0usize;
        for path in candidates {
            let rel_for_match = path
                .strip_prefix(&resolved_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string()
                .replace('\\', "/");
            if !matcher.is_match(&rel_for_match) {
                continue;
            }

            let relative = path
                .strip_prefix(&workspace_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            rendered.push_str(&relative);
            rendered.push('\n');
            count += 1;
            if count >= options.max_list_entries {
                break;
            }
        }

        Ok(rendered)
    }

    pub(crate) async fn agent_tool_grep(
        &self,
        pattern: &str,
        root: Option<&str>,
        include: Option<&str>,
        context: &CommandContext,
        options: &AgentOptions,
    ) -> Result<String, ExecutionError> {
        let root = root.unwrap_or(".");
        let resolved_root = self.resolve_workspace_path(context, root, PathOperation::List)?;
        let workspace_root = path_utils::absolute_normalized(&context.config.workspace_root)
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let re = Regex::new(pattern)
            .map_err(|err| ExecutionError::Dispatch(format!("invalid regex pattern: {err}")))?;
        let include_matcher = if let Some(include) = include {
            Some(
                Glob::new(include)
                    .map_err(|err| {
                        ExecutionError::Dispatch(format!("invalid include glob: {err}"))
                    })?
                    .compile_matcher(),
            )
        } else {
            None
        };

        let candidates = self
            .fs
            .walk_dir_limited(&resolved_root, options.max_list_entries)
            .await
            .map_err(|err| ExecutionError::Executor(err.to_string()))?;

        let mut rendered = String::new();
        let mut matches = 0usize;
        for path in candidates {
            let meta = self
                .fs
                .metadata(&path)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?;
            if !meta.is_file {
                continue;
            }

            let relative = path
                .strip_prefix(&workspace_root)
                .unwrap_or(path.as_path())
                .display()
                .to_string();
            if let Some(include) = include_matcher.as_ref() {
                let normalized = relative.replace('\\', "/");
                if !include.is_match(&normalized) {
                    continue;
                }
            }

            let content = self
                .fs
                .read_to_string_limited(&path, options.max_read_bytes)
                .await
                .map_err(|err| ExecutionError::Executor(err.to_string()))?;

            for (idx, line) in content.lines().enumerate() {
                if !re.is_match(line) {
                    continue;
                }
                let line_no = idx + 1;
                rendered.push_str(&relative);
                rendered.push(':');
                rendered.push_str(&line_no.to_string());
                rendered.push_str(": ");
                rendered.push_str(line);
                rendered.push('\n');
                matches += 1;
                if matches >= 2000 {
                    return Ok(rendered);
                }
            }
        }

        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::diff_output;

    #[test]
    fn diff_output_includes_two_lines_of_context_for_single_line_change() {
        let old = "line1\nline2\nline3\nline4\nline5\nline6\n";
        let new = "line1\nline2\nCHANGED\nline4\nline5\nline6\n";
        let out = diff_output(old, new, 40);

        assert!(out.contains("@@ -1,5 +1,5 @@"), "out={out}");
        assert!(out.contains(" line1"), "out={out}");
        assert!(out.contains(" line2"), "out={out}");
        assert!(out.contains("-line3"), "out={out}");
        assert!(out.contains("+CHANGED"), "out={out}");
        assert!(out.contains(" line4"), "out={out}");
        assert!(out.contains(" line5"), "out={out}");
    }

    #[test]
    fn diff_output_truncates_changed_lines_budget_only() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "A\nB\nC\nD\ne\nf\n";
        let out = diff_output(old, new, 2);

        assert!(out.contains("+4 -4"), "out={out}");
        assert!(out.contains("...[6 more diff lines]"), "out={out}");
    }
}
