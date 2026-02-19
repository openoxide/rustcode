use rustcode_core::context::CommandContext;
use rustcode_snapshot::SnapshotStore;

use super::ExecutionError;
use crate::{AgentState, Engine};

impl Engine {
    /// Take an automatic snapshot of the workspace before the first mutation.
    ///
    /// If a snapshot has already been taken for this agent run (tracked via
    /// `state.auto_snapshot_hash`), this is a no-op.  The snapshot hash is
    /// stored in the state so that `snapshot_list` / `snapshot_restore` tools
    /// can reference it.
    ///
    /// Snapshot failures are non-fatal: a warning is logged and the mutation
    /// proceeds without a snapshot.
    pub(crate) async fn auto_snapshot_before_mutation(
        &self,
        state: &mut AgentState,
        context: &CommandContext,
    ) {
        if state.auto_snapshot_hash.is_some() {
            return; // already snapshotted for this run
        }

        match open_snapshot_store(context) {
            None => {} // no data dir — skip silently
            Some(store) => {
                if let Err(e) = store.ensure_init().await {
                    tracing::warn!("snapshot init failed: {e}");
                    return;
                }
                match store.track().await {
                    Ok(hash) => {
                        tracing::debug!(%hash, "auto-snapshot taken before first mutation");
                        state.auto_snapshot_hash = Some(hash);
                    }
                    Err(e) => {
                        tracing::warn!("snapshot track failed: {e}");
                    }
                }
            }
        }
    }

    /// List recent snapshots for the current session.
    ///
    /// Returns a formatted list of SHA-1 hashes that can be passed to
    /// `snapshot_restore`.  Returns an informational message when no snapshots
    /// exist yet.
    ///
    /// # Errors
    /// Returns `ExecutionError::Executor` if listing the object store fails.
    pub(crate) async fn agent_tool_snapshot_list(
        &self,
        context: &CommandContext,
        state: &AgentState,
    ) -> Result<String, ExecutionError> {
        let Some(store) = open_snapshot_store(context) else {
            return Ok("Snapshot storage directory unavailable (HOME not set). \
                 Snapshots cannot be used in this environment."
                .to_string());
        };

        if let Err(e) = store.ensure_init().await {
            return Err(ExecutionError::Executor(format!(
                "snapshot store init failed: {e}"
            )));
        }

        let hashes = store
            .list()
            .await
            .map_err(|e| ExecutionError::Executor(format!("snapshot list failed: {e}")))?;

        if hashes.is_empty() {
            let msg = if state.auto_snapshot_hash.is_none() {
                "No snapshots recorded yet. Snapshots are taken automatically before the \
                 first file write or edit in an agent session."
            } else {
                "Snapshot taken but no objects found in the store yet."
            };
            return Ok(msg.to_string());
        }

        let mut output = format!("{} snapshot(s) available:\n\n", hashes.len());
        // Show the auto-snapshot hash first with a label
        if let Some(auto) = &state.auto_snapshot_hash {
            output.push_str(&format!("  [auto] {auto}\n"));
        }
        for (i, hash) in hashes.iter().enumerate() {
            let label = if Some(hash) == state.auto_snapshot_hash.as_ref() {
                " ← auto"
            } else {
                ""
            };
            output.push_str(&format!("  {}: {}{}\n", i + 1, hash, label));
        }
        output.push_str(
            "\nPass any hash to `snapshot_restore` to revert the workspace to that state.",
        );
        Ok(output)
    }

    /// Restore the workspace to a previously recorded snapshot.
    ///
    /// Uses `git read-tree` + `git checkout-index -a -f` to restore all files
    /// that existed at snapshot time.  Files added after the snapshot are left
    /// in place.
    ///
    /// # Errors
    /// Returns `ExecutionError::Dispatch` if the hash is empty.
    /// Returns `ExecutionError::Executor` if the git restore operation fails.
    pub(crate) async fn agent_tool_snapshot_restore(
        &self,
        hash: &str,
        context: &CommandContext,
    ) -> Result<String, ExecutionError> {
        let hash = hash.trim();
        if hash.is_empty() {
            return Err(ExecutionError::Dispatch(
                "snapshot_restore requires a non-empty snapshot hash".to_string(),
            ));
        }

        let Some(store) = open_snapshot_store(context) else {
            return Err(ExecutionError::Dispatch(
                "snapshot storage directory unavailable (HOME not set)".to_string(),
            ));
        };

        if let Err(e) = store.ensure_init().await {
            return Err(ExecutionError::Executor(format!(
                "snapshot store init failed: {e}"
            )));
        }

        store
            .restore(hash)
            .await
            .map_err(|e| ExecutionError::Executor(format!("snapshot restore failed: {e}")))?;

        Ok(format!(
            "Workspace successfully restored to snapshot {hash}."
        ))
    }
}

/// Build a [`SnapshotStore`] for the current session from the context.
///
/// Returns `None` when the snapshot storage directory cannot be determined
/// (e.g. `HOME` is not set in the environment).
fn open_snapshot_store(context: &CommandContext) -> Option<SnapshotStore> {
    SnapshotStore::new(&context.session.session_id, &context.config.workspace_root).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    use rustcode_core::config::ResolvedConfig;
    use rustcode_core::context::{CommandContext, SessionMeta};

    use super::*;

    fn make_context(session_id: &str, workspace: &std::path::Path) -> CommandContext {
        use rustcode_core::config::BackendSelectionPolicy;
        let cfg = ResolvedConfig {
            profile: "test".to_string(),
            workspace_root: workspace.to_path_buf(),
            allow_network: false,
            model: "test-model".to_string(),
            llm_provider: "test".to_string(),
            llm_base_url: None,
            llm_api_key_env: None,
            enabled_providers: None,
            disabled_providers: Default::default(),
            mcp_servers: Default::default(),
            plugins: vec![],
            env: Default::default(),
            backend_selection: BackendSelectionPolicy {
                provider_agnostic_weight: 0,
                automation_skills_weight: 0,
                open_source_weight: 0,
                lsp_support_weight: 0,
                privacy_weight: 0,
                subscription_penalty: 0,
            },
            permission_rules: vec![],
            project_config_path: None,
            project_config_trusted: false,
        };
        CommandContext::new(
            Arc::new(cfg),
            SessionMeta {
                session_id: session_id.to_string(),
                request_id: "req-1".to_string(),
                started_at: SystemTime::now(),
            },
        )
    }

    #[test]
    fn open_snapshot_store_returns_some_with_valid_context() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = make_context("snap-test-session", tmp.path());
        let store = open_snapshot_store(&ctx);
        // Requires HOME to be set — will be Some in normal CI
        assert!(store.is_some() || std::env::var("HOME").is_err());
    }
}
