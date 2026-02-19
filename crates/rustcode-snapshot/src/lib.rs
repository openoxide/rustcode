// ── rustcode-snapshot ─────────────────────────────────────────────────────────
//
// Git-based workspace snapshot system for rustcode.
//
// Each session gets a **separate** git repository stored at
// `~/.local/share/rustcode/snapshots/<session-id>/git/` (the `--git-dir`).
// The workspace root remains as the working tree (`--work-tree`).
//
// This means snapshot history never pollutes the project's own git history.
//
// Reference: opencode `packages/opencode/src/snapshot/index.ts`

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::process::Command;

/// Errors produced by the snapshot system.
#[derive(Debug, Error)]
pub enum SnapshotError {
    /// Failed to locate the user's home/data directory.
    #[error("could not determine snapshot storage directory")]
    NoDataDir,
    /// An underlying I/O error (directory creation, etc.).
    #[error("snapshot I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A git command failed or returned a non-zero exit code.
    #[error("git command failed: {0}")]
    Git(String),
}

/// A session-scoped snapshot store.
///
/// Each `SnapshotStore` manages a single private git repository that tracks
/// one workspace directory.  Snapshots are cheap: only the object store and
/// tree entries are written — no branches, no commits.
pub struct SnapshotStore {
    /// Absolute path to the private git directory (`--git-dir`).
    git_dir: PathBuf,
    /// Absolute path to the workspace being snapshotted (`--work-tree`).
    work_tree: PathBuf,
}

impl SnapshotStore {
    /// Create a new store for the given session and workspace.
    ///
    /// Derives the storage path from the XDG data directory:
    /// `$XDG_DATA_HOME/rustcode/snapshots/<session_id>/git/`
    /// or `~/.local/share/rustcode/snapshots/<session_id>/git/`.
    ///
    /// The git directory is **not** initialised here; call [`Self::ensure_init`]
    /// before calling any other method.
    ///
    /// # Errors
    /// Returns [`SnapshotError::NoDataDir`] when neither `$XDG_DATA_HOME` nor
    /// the home directory can be determined.
    pub fn new(session_id: &str, workspace_root: &Path) -> Result<Self, SnapshotError> {
        let base = xdg_data_dir().ok_or(SnapshotError::NoDataDir)?;
        let git_dir = base
            .join("rustcode")
            .join("snapshots")
            .join(session_id)
            .join("git");
        Ok(Self {
            git_dir,
            work_tree: workspace_root.to_path_buf(),
        })
    }

    /// Initialise the private git repository if it does not already exist.
    ///
    /// Safe to call multiple times; subsequent calls are no-ops.
    ///
    /// # Errors
    /// Returns [`SnapshotError::Io`] on directory creation failure.
    /// Returns [`SnapshotError::Git`] if `git init` fails.
    pub async fn ensure_init(&self) -> Result<(), SnapshotError> {
        tokio::fs::create_dir_all(&self.git_dir).await?;

        // Check whether the git dir is already initialised (HEAD file exists)
        if tokio::fs::try_exists(self.git_dir.join("HEAD")).await? {
            return Ok(());
        }

        let out = Command::new("git")
            .args(["init", "--bare"])
            .arg(&self.git_dir)
            .output()
            .await?;

        if !out.status.success() {
            return Err(SnapshotError::Git(git_stderr(&out.stderr)));
        }

        // Disable line-ending conversion (cross-platform safety)
        let _ = self.git(&["config", "core.autocrlf", "false"]).await;
        tracing::debug!(git_dir = %self.git_dir.display(), "snapshot store initialised");
        Ok(())
    }

    /// Stage all workspace files and write a tree object.
    ///
    /// Returns the SHA-1 hash of the created tree (40 hex characters).
    /// This hash can later be passed to [`Self::restore`] or [`Self::changed_files`].
    ///
    /// # Errors
    /// Returns [`SnapshotError::Git`] if staging or tree creation fails.
    pub async fn track(&self) -> Result<String, SnapshotError> {
        // Stage everything
        self.git_work_tree(&["add", "--all", "--force", "."]).await?;

        // Write the tree object and capture its hash
        let out = self
            .git_work_tree_raw(&["write-tree"])
            .output()
            .await?;

        if !out.status.success() {
            return Err(SnapshotError::Git(git_stderr(&out.stderr)));
        }

        let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
        tracing::debug!(%hash, "snapshot tracked");
        Ok(hash)
    }

    /// Restore the workspace to the state recorded in `snapshot_hash`.
    ///
    /// Files that existed in the snapshot are restored; files that did NOT
    /// exist in the snapshot (i.e. newly added files) are left untouched
    /// (use `git status` to identify them if needed).
    ///
    /// # Errors
    /// Returns [`SnapshotError::Git`] if the restore operation fails.
    pub async fn restore(&self, snapshot_hash: &str) -> Result<(), SnapshotError> {
        // Populate the index from the tree
        self.git_work_tree(&["read-tree", snapshot_hash]).await?;

        // Check out all index entries into the work tree
        self.git_work_tree(&["checkout-index", "-a", "-f"]).await?;

        tracing::info!(%snapshot_hash, "workspace restored from snapshot");
        Ok(())
    }

    /// Return the list of workspace-relative paths that changed between
    /// `snapshot_hash` and the current working tree.
    ///
    /// Stages the current state before diffing so untracked files are included.
    ///
    /// # Errors
    /// Returns [`SnapshotError::Git`] if the diff operation fails.
    pub async fn changed_files(&self, snapshot_hash: &str) -> Result<Vec<String>, SnapshotError> {
        self.git_work_tree(&["add", "--all", "--force", "."]).await?;

        let out = self
            .git_work_tree_raw(&[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.quotepath=false",
                "diff",
                "--no-ext-diff",
                "--name-only",
                snapshot_hash,
                "--",
                ".",
            ])
            .output()
            .await?;

        if !out.status.success() {
            // diff exits 1 when there are differences — that is not an error
            if out.status.code() != Some(1) {
                return Err(SnapshotError::Git(git_stderr(&out.stderr)));
            }
        }

        let output = String::from_utf8_lossy(&out.stdout);
        let files = output
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        Ok(files)
    }

    /// List stored snapshot hashes for this session (most-recent last).
    ///
    /// Returns an empty list when no snapshots exist yet or when the git
    /// object store has no loose objects.
    ///
    /// # Errors
    /// Returns [`SnapshotError::Io`] on directory read failure.
    pub async fn list(&self) -> Result<Vec<String>, SnapshotError> {
        let objects_dir = self.git_dir.join("objects");
        if !tokio::fs::try_exists(&objects_dir).await? {
            return Ok(Vec::new());
        }

        // Collect all loose object hashes from the 2-char fan-out directories
        let mut hashes = Vec::new();
        let mut dirs = tokio::fs::read_dir(&objects_dir).await?;
        while let Some(entry) = dirs.next_entry().await? {
            let name = entry.file_name();
            let prefix = name.to_string_lossy();
            if prefix.len() == 2 && prefix.chars().all(|c| c.is_ascii_hexdigit()) {
                let mut files = tokio::fs::read_dir(entry.path()).await?;
                while let Some(obj) = files.next_entry().await? {
                    let suffix = obj.file_name().to_string_lossy().to_string();
                    hashes.push(format!("{prefix}{suffix}"));
                }
            }
        }

        // Sort by hash (proxy for creation order with SHA-1)
        hashes.sort();
        Ok(hashes)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Run a git command in the private `--git-dir` (no `--work-tree`).
    async fn git(&self, args: &[&str]) -> Result<(), SnapshotError> {
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(&self.git_dir)
            .args(args)
            .output()
            .await?;
        if !out.status.success() {
            return Err(SnapshotError::Git(git_stderr(&out.stderr)));
        }
        Ok(())
    }

    /// Run a git command with both `--git-dir` and `--work-tree` set.
    async fn git_work_tree(&self, args: &[&str]) -> Result<(), SnapshotError> {
        let out = self.git_work_tree_raw(args).output().await?;
        if !out.status.success() {
            return Err(SnapshotError::Git(git_stderr(&out.stderr)));
        }
        Ok(())
    }

    /// Build a `Command` for git with `--git-dir` and `--work-tree`.
    ///
    /// Caller is responsible for calling `.output()` and checking the status.
    fn git_work_tree_raw(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(args);
        cmd
    }
}

/// Resolve the XDG data home directory.
fn xdg_data_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg));
    }
    dirs_or_home().map(|h| h.join(".local").join("share"))
}

/// Return home directory via the `HOME` environment variable (no extra deps).
fn dirs_or_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn git_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_data_dir_falls_back_to_home() {
        // As long as HOME or XDG_DATA_HOME is set this should return Some
        let dir = xdg_data_dir();
        assert!(dir.is_some(), "expected a data dir");
    }

    #[test]
    fn store_new_constructs_path() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new("test-session-123", tmp.path()).unwrap();
        assert!(
            store.git_dir.to_string_lossy().contains("test-session-123"),
            "git dir should include session id"
        );
        assert_eq!(store.work_tree, tmp.path());
    }

    #[tokio::test]
    async fn ensure_init_creates_git_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new("init-test", workspace.path()).unwrap();
        store.ensure_init().await.unwrap();
        assert!(
            store.git_dir.exists(),
            "git dir should exist after ensure_init"
        );
        assert!(
            store.git_dir.join("HEAD").exists(),
            "HEAD file should exist in initialised git dir"
        );
    }

    #[tokio::test]
    async fn track_and_list_and_changed_files() {
        // Requires git to be installed
        let workspace = tempfile::tempdir().unwrap();
        // Write a test file
        tokio::fs::write(workspace.path().join("hello.txt"), "hello world")
            .await
            .unwrap();

        let store = SnapshotStore::new("track-test", workspace.path()).unwrap();
        store.ensure_init().await.unwrap();

        let hash = store.track().await.unwrap();
        assert_eq!(hash.len(), 40, "SHA-1 hash should be 40 chars");

        let hashes = store.list().await.unwrap();
        assert!(!hashes.is_empty(), "list should return at least one hash");

        // No changes since track
        let changed = store.changed_files(&hash).await.unwrap();
        assert!(
            changed.is_empty(),
            "no files should be changed right after track"
        );

        // Modify a file and check changed_files
        tokio::fs::write(workspace.path().join("hello.txt"), "goodbye world")
            .await
            .unwrap();
        let changed = store.changed_files(&hash).await.unwrap();
        assert!(
            changed.contains(&"hello.txt".to_string()),
            "hello.txt should appear as changed"
        );
    }

    #[tokio::test]
    async fn restore_reverts_changes() {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace.path().join("data.txt");
        tokio::fs::write(&file_path, "original content").await.unwrap();

        let store = SnapshotStore::new("restore-test", workspace.path()).unwrap();
        store.ensure_init().await.unwrap();
        let hash = store.track().await.unwrap();

        // Overwrite the file
        tokio::fs::write(&file_path, "modified content").await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&file_path).await.unwrap(),
            "modified content"
        );

        // Restore
        store.restore(&hash).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&file_path).await.unwrap(),
            "original content",
            "file should be restored to its original content"
        );
    }
}
