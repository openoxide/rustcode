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
fn diff_output(old: &str, new: &str, max_diff_lines: usize) -> String {
    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut diff_lines: Vec<String> = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => {
                added += 1;
                if diff_lines.len() < max_diff_lines {
                    let line = change.value().trim_end_matches('\n');
                    diff_lines.push(format!("+{line}"));
                }
            }
            ChangeTag::Delete => {
                removed += 1;
                if diff_lines.len() < max_diff_lines {
                    let line = change.value().trim_end_matches('\n');
                    diff_lines.push(format!("-{line}"));
                }
            }
            ChangeTag::Equal => {}
        }
    }
    if added == 0 && removed == 0 {
        return String::new();
    }
    let truncated = if added + removed > max_diff_lines {
        format!(
            "\n...[{} more diff lines]",
            (added + removed).saturating_sub(diff_lines.len())
        )
    } else {
        String::new()
    };
    format!(
        "+{added} -{removed}\n@@diff\n{}{}",
        diff_lines.join("\n"),
        truncated
    )
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
