// ── Git worktree agent tool handlers ───────────────────────────────────────
//
// All operations shell out to `git` via the existing `ProcessPort::run_capture`.
// No git library dependency is needed — the operations are simple and the
// subprocess approach keeps the crate lean.
//
// Reference: opencode `packages/opencode/src/worktree/index.ts`.

use std::time::Duration;

use super::{CommandContext, Engine, ExecutionError, IoError};

/// Timeout for all worktree git operations.
const WORKTREE_TIMEOUT_SECS: u64 = 60;

/// Run a git subcommand in the workspace root; return stdout on exit-0,
/// error (with stderr) on non-zero.
async fn git(
    engine: &Engine,
    args: &[&str],
    context: &CommandContext,
) -> Result<String, ExecutionError> {
    let arg_strings: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let output = tokio::time::timeout(
        Duration::from_secs(WORKTREE_TIMEOUT_SECS),
        engine.process.run_capture(
            "git",
            &arg_strings,
            &context.config.workspace_root,
            context.cancellation.clone(),
        ),
    )
    .await
    .map_err(|_| {
        ExecutionError::Executor(format!(
            "git {} timed out after {WORKTREE_TIMEOUT_SECS}s",
            args.join(" ")
        ))
    })?
    .map_err(|e| match e {
        IoError::Cancelled => ExecutionError::Cancelled,
        _ => ExecutionError::Executor(e.to_string()),
    })?;

    if output.code != 0 {
        let msg = if output.stderr.is_empty() {
            format!("git {} exited with code {}", args.join(" "), output.code)
        } else {
            output.stderr.trim().to_string()
        };
        return Err(ExecutionError::Executor(msg));
    }
    Ok(output.stdout)
}

/// Generate a worktree name from the current UTC timestamp if none is provided.
fn default_worktree_name() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("wt-{ts}")
}

impl Engine {
    /// Create a new git worktree.
    ///
    /// Creates a sibling directory to the workspace root with the given `name`
    /// (or a timestamp-based name), checked out on a new branch `branch`
    /// (defaults to the worktree name).
    ///
    /// Gated on `allow_exec` in the caller.
    pub(crate) async fn agent_tool_worktree_create(
        &self,
        name: Option<&str>,
        branch: Option<&str>,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let wt_name = name.map_or_else(default_worktree_name, str::to_string);
        let wt_branch = branch.map_or_else(|| wt_name.clone(), str::to_string);

        // Resolve path as sibling of workspace root
        let parent =
            context.config.workspace_root.parent().ok_or_else(|| {
                ExecutionError::Executor("workspace root has no parent".to_string())
            })?;
        let wt_path = parent.join(&wt_name);
        let wt_path_str = wt_path.to_str().ok_or_else(|| {
            ExecutionError::Executor("worktree path is not valid UTF-8".to_string())
        })?;

        // git worktree add -b <branch> <path>
        git(
            self,
            &["worktree", "add", "-b", &wt_branch, wt_path_str],
            context,
        )
        .await?;

        Ok(format!(
            "worktree created\nname: {wt_name}\nbranch: {wt_branch}\npath: {wt_path_str}"
        ))
    }

    /// List all git worktrees as a formatted table.
    ///
    /// Parses `git worktree list --porcelain` output.
    pub(crate) async fn agent_tool_worktree_list(
        &self,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let raw = git(self, &["worktree", "list", "--porcelain"], context).await?;
        let worktrees = parse_worktree_porcelain(&raw);
        if worktrees.is_empty() {
            return Ok("no worktrees".to_string());
        }
        let mut out = String::from("worktrees:\n");
        for wt in &worktrees {
            out.push_str(&format!(
                "  path={} branch={} bare={}\n",
                wt.path, wt.branch, wt.bare
            ));
        }
        Ok(out.trim_end().to_string())
    }

    /// Remove a git worktree at the given path.
    ///
    /// Uses `--force` to handle unclean worktrees.
    pub(crate) async fn agent_tool_worktree_remove(
        &self,
        path: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if path.is_empty() {
            return Err(ExecutionError::Dispatch(
                "worktree_remove requires a non-empty path".to_string(),
            ));
        }
        git(self, &["worktree", "remove", "--force", path], context).await?;
        Ok(format!("worktree removed: {path}"))
    }

    /// Hard-reset a worktree to its remote tracking branch.
    ///
    /// Runs inside the target worktree directory:
    ///   1. `git fetch --prune`
    ///   2. `git reset --hard @{u}` (upstream branch)
    ///   3. `git clean -ffdx` (remove untracked + ignored files)
    pub(crate) async fn agent_tool_worktree_reset(
        &self,
        path: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        if path.is_empty() {
            return Err(ExecutionError::Dispatch(
                "worktree_reset requires a non-empty path".to_string(),
            ));
        }
        // Run all three git commands with -C <path> so we target the worktree dir
        let fetch_out = tokio::time::timeout(
            Duration::from_secs(WORKTREE_TIMEOUT_SECS),
            self.process.run_capture(
                "git",
                &[
                    "-C".to_string(),
                    path.to_string(),
                    "fetch".to_string(),
                    "--prune".to_string(),
                ],
                &context.config.workspace_root,
                context.cancellation.clone(),
            ),
        )
        .await
        .map_err(|_| ExecutionError::Executor(format!("git fetch timed out for {path}")))?
        .map_err(|e| match e {
            IoError::Cancelled => ExecutionError::Cancelled,
            _ => ExecutionError::Executor(e.to_string()),
        })?;

        if fetch_out.code != 0 {
            return Err(ExecutionError::Executor(
                fetch_out.stderr.trim().to_string(),
            ));
        }

        let reset_out = tokio::time::timeout(
            Duration::from_secs(WORKTREE_TIMEOUT_SECS),
            self.process.run_capture(
                "git",
                &[
                    "-C".to_string(),
                    path.to_string(),
                    "reset".to_string(),
                    "--hard".to_string(),
                    "@{u}".to_string(),
                ],
                &context.config.workspace_root,
                context.cancellation.clone(),
            ),
        )
        .await
        .map_err(|_| ExecutionError::Executor(format!("git reset timed out for {path}")))?
        .map_err(|e| match e {
            IoError::Cancelled => ExecutionError::Cancelled,
            _ => ExecutionError::Executor(e.to_string()),
        })?;

        if reset_out.code != 0 {
            return Err(ExecutionError::Executor(
                reset_out.stderr.trim().to_string(),
            ));
        }

        let clean_out = tokio::time::timeout(
            Duration::from_secs(WORKTREE_TIMEOUT_SECS),
            self.process.run_capture(
                "git",
                &[
                    "-C".to_string(),
                    path.to_string(),
                    "clean".to_string(),
                    "-ffdx".to_string(),
                ],
                &context.config.workspace_root,
                context.cancellation.clone(),
            ),
        )
        .await
        .map_err(|_| ExecutionError::Executor(format!("git clean timed out for {path}")))?
        .map_err(|e| match e {
            IoError::Cancelled => ExecutionError::Cancelled,
            _ => ExecutionError::Executor(e.to_string()),
        })?;

        if clean_out.code != 0 {
            return Err(ExecutionError::Executor(
                clean_out.stderr.trim().to_string(),
            ));
        }

        Ok(format!("worktree reset to upstream: {path}"))
    }
}

/// Parsed representation of one entry from `git worktree list --porcelain`.
struct WorktreeEntry {
    path: String,
    branch: String,
    bare: bool,
}

/// Parse `git worktree list --porcelain` output into a vec of entries.
///
/// Porcelain format: blank-line separated stanzas, each with:
/// ```text
/// worktree /path/to/wt
/// HEAD <sha>
/// branch refs/heads/<name>   (or "detached" / absent for bare)
/// bare                       (optional)
/// ```
fn parse_worktree_porcelain(raw: &str) -> Vec<WorktreeEntry> {
    let mut result = Vec::new();
    let mut path = String::new();
    let mut branch = String::new();
    let mut bare = false;

    for line in raw.lines() {
        if line.is_empty() {
            if !path.is_empty() {
                result.push(WorktreeEntry {
                    path: std::mem::take(&mut path),
                    branch: if branch.is_empty() {
                        "detached".to_string()
                    } else {
                        std::mem::take(&mut branch)
                    },
                    bare,
                });
                bare = false;
            }
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = b.to_string();
        } else if line == "bare" {
            bare = true;
        }
    }
    // Handle last stanza if file doesn't end with blank line
    if !path.is_empty() {
        result.push(WorktreeEntry {
            path,
            branch: if branch.is_empty() {
                "detached".to_string()
            } else {
                branch
            },
            bare,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_two_worktrees() {
        let raw = "worktree /home/user/project\n\
                   HEAD abc123\n\
                   branch refs/heads/main\n\
                   \n\
                   worktree /home/user/wt-1\n\
                   HEAD def456\n\
                   branch refs/heads/feature\n\
                   ";
        let entries = parse_worktree_porcelain(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/home/user/project");
        assert_eq!(entries[0].branch, "main");
        assert!(!entries[0].bare);
        assert_eq!(entries[1].path, "/home/user/wt-1");
        assert_eq!(entries[1].branch, "feature");
    }

    #[test]
    fn parse_porcelain_bare() {
        let raw = "worktree /bare/repo\nHEAD abc\nbranch refs/heads/main\nbare\n";
        let entries = parse_worktree_porcelain(raw);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].bare);
    }

    #[test]
    fn parse_porcelain_detached() {
        let raw = "worktree /detached\nHEAD abc123\ndetached\n";
        let entries = parse_worktree_porcelain(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch, "detached");
    }

    #[test]
    fn default_name_has_prefix() {
        let name = default_worktree_name();
        assert!(name.starts_with("wt-"), "expected wt- prefix, got: {name}");
    }
}
