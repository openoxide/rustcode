use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("io error: {0}")]
    Io(String),
    #[error("process exited with code {code}: {stderr}")]
    Exit { code: i32, stderr: String },
    #[error("operation cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait FileSystemPort: Send + Sync {
    async fn read_to_string(&self, path: &Path) -> Result<String, IoError>;
    async fn read_to_string_limited(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Result<String, IoError>;
    async fn exists(&self, path: &Path) -> Result<bool, IoError>;
    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), IoError>;
    async fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>, IoError>;
    async fn list_dir_limited(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError>;

    async fn metadata(&self, path: &Path) -> Result<FsMetadata, IoError>;

    /// Recursively walks a directory tree and returns entries in deterministic order.
    ///
    /// Implementations must not follow symlinks.
    async fn walk_dir_limited(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsMetadata {
    pub is_dir: bool,
    pub is_file: bool,
    pub len: u64,
}

#[async_trait]
pub trait ProcessPort: Send + Sync {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError>;

    /// Runs a process and captures output regardless of exit status.
    ///
    /// This is useful for tool execution where stdout/stderr should be returned to the caller
    /// even when the command fails.
    async fn run_capture(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError>;
}

#[derive(Debug, Default)]
pub struct LocalIo;

#[async_trait]
impl FileSystemPort for LocalIo {
    async fn read_to_string(&self, path: &Path) -> Result<String, IoError> {
        tokio::fs::read_to_string(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))
    }

    async fn read_to_string_limited(
        &self,
        path: &Path,
        max_bytes: usize,
    ) -> Result<String, IoError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?;

        // Read at most max_bytes + 1 to detect truncation without allocating the full file.
        let mut buf = Vec::new();
        let mut limited = (&mut file).take((max_bytes.saturating_add(1)) as u64);
        limited
            .read_to_end(&mut buf)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?;

        let truncated = buf.len() > max_bytes;
        if truncated {
            buf.truncate(max_bytes);
        }

        let mut text = String::from_utf8_lossy(&buf).to_string();
        if truncated {
            // Marker is intentionally machine-detectable so higher layers can refuse unsafe edits.
            text.push_str("\n[rustcode:truncated]\n");
        }
        Ok(text)
    }

    async fn exists(&self, path: &Path) -> Result<bool, IoError> {
        tokio::fs::try_exists(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))
    }

    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), IoError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| IoError::Io(format!("{}: {err}", parent.display())))?;
        }
        tokio::fs::write(path, contents)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))
    }

    async fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>, IoError> {
        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?;
        let mut paths = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?
        {
            paths.push(entry.path());
        }

        paths.sort();
        Ok(paths)
    }

    async fn list_dir_limited(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?;
        let mut paths = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?
        {
            paths.push(entry.path());
            if paths.len() >= max_entries {
                break;
            }
        }

        paths.sort();
        Ok(paths)
    }

    async fn metadata(&self, path: &Path) -> Result<FsMetadata, IoError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|err| IoError::Io(format!("{}: {err}", path.display())))?;
        Ok(FsMetadata {
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            len: metadata.len(),
        })
    }

    async fn walk_dir_limited(
        &self,
        path: &Path,
        max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        let mut out = Vec::new();
        let mut queue = vec![path.to_path_buf()];

        while let Some(dir) = queue.pop() {
            let mut entries = tokio::fs::read_dir(&dir)
                .await
                .map_err(|err| IoError::Io(format!("{}: {err}", dir.display())))?;

            let mut paths = Vec::new();
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|err| IoError::Io(format!("{}: {err}", dir.display())))?
            {
                paths.push(entry);
            }

            // Deterministic traversal: sort by full path and always enqueue subdirs after.
            paths.sort_by_key(tokio::fs::DirEntry::path);
            for entry in paths {
                let entry_path = entry.path();
                out.push(entry_path.clone());
                if out.len() >= max_entries {
                    out.sort();
                    return Ok(out);
                }

                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|err| IoError::Io(format!("{}: {err}", entry_path.display())))?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    queue.push(entry_path);
                }
            }
        }

        out.sort();
        Ok(out)
    }
}

#[async_trait]
impl ProcessPort for LocalIo {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        let output = self.run_capture(program, args, cwd, cancellation).await?;

        if output.code == 0 {
            Ok(output)
        } else {
            Err(IoError::Exit {
                code: output.code,
                stderr: output.stderr,
            })
        }
    }

    async fn run_capture(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        let mut command = Command::new(program);
        command.args(args);
        command.current_dir(cwd);
        command.kill_on_drop(true);

        let output = tokio::select! {
            result = command.output() => {
                result.map_err(|err| IoError::Io(format!("{program}: {err}")))?
            }
            () = cancellation.cancelled() => {
                return Err(IoError::Cancelled);
            }
        };

        Ok(ProcessOutput {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

#[must_use]
pub fn normalize_path(root: &Path, candidate: &Path) -> PathBuf {
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn process_run_can_be_cancelled() {
        let io = LocalIo;
        let token = CancellationToken::new();
        let token_for_task = token.clone();

        let task = tokio::spawn(async move {
            io.run(
                "sleep",
                &["30".to_string()],
                Path::new("/tmp"),
                token_for_task,
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        token.cancel();

        let result = task.await.expect("task must join");
        assert!(matches!(result, Err(IoError::Cancelled)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_run_returns_stdout_on_success() {
        let io = LocalIo;
        let output = io
            .run(
                "echo",
                &["hello".to_string()],
                Path::new("/tmp"),
                CancellationToken::new(),
            )
            .await
            .expect("must succeed");

        assert_eq!(output.code, 0);
        assert_eq!(output.stdout, "hello\n");
    }
}
