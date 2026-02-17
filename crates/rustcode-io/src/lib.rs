use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;
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
    async fn write_string(&self, path: &Path, contents: &str) -> Result<(), IoError>;
    async fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>, IoError>;
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
        let mut command = Command::new(program);
        command.args(args);
        command.current_dir(cwd);
        command.kill_on_drop(true);

        let output = tokio::select! {
            result = command.output() => {
                result.map_err(|err| IoError::Io(format!("{program}: {err}")))?
            }
            _ = cancellation.cancelled() => {
                return Err(IoError::Cancelled);
            }
        };

        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(ProcessOutput {
                code,
                stdout,
                stderr,
            })
        } else {
            Err(IoError::Exit { code, stderr })
        }
    }
}

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
