use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("io error: {0}")]
    Io(String),
    #[error("process exited with code {code}: {stderr}")]
    Exit { code: i32, stderr: String },
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
}

#[async_trait]
pub trait ProcessPort: Send + Sync {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
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
}

#[async_trait]
impl ProcessPort for LocalIo {
    async fn run(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
    ) -> Result<ProcessOutput, IoError> {
        let mut command = Command::new(program);
        command.args(args);
        command.current_dir(cwd);
        let output = command
            .output()
            .await
            .map_err(|err| IoError::Io(format!("{program}: {err}")))?;

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
