//! Unified error hierarchy for the rustcode project.
//!
//! This crate re-exports the foundational error types from [`rustcode_core`]
//! and adds higher-level error types for IO, LLM, and MCP operations.
//! Use [`RustcodeError`] as the umbrella type when you need to unify errors
//! from multiple subsystems.

use thiserror::Error;

// ── Re-exports from rustcode-core ──────────────────────────────────────
pub use rustcode_core::error::{ConfigError, ExecutionError, PublishError, ShutdownError};

// ── IO errors ──────────────────────────────────────────────────────────

/// Errors from file-system and process operations.
#[derive(Debug, Error)]
pub enum IoError {
    /// A file could not be read.
    #[error("file read failed: {path}: {reason}")]
    ReadFailed {
        /// The path that was being read.
        path: String,
        /// Why the read failed.
        reason: String,
    },

    /// A file could not be written.
    #[error("file write failed: {path}: {reason}")]
    WriteFailed {
        /// The path that was being written.
        path: String,
        /// Why the write failed.
        reason: String,
    },

    /// A path is outside the allowed workspace.
    #[error("path escapes workspace root: {path}")]
    PathEscape {
        /// The offending path.
        path: String,
    },

    /// A subprocess exited with a non-zero code.
    #[error("process exited with code {code}: {stderr}")]
    ProcessFailed {
        /// Exit code (or -1 if killed by signal).
        code: i32,
        /// Captured stderr output.
        stderr: String,
    },

    /// A generic OS-level I/O error.
    #[error("io error: {0}")]
    Os(#[from] std::io::Error),
}

// ── LLM errors ─────────────────────────────────────────────────────────

/// Errors from LLM provider interactions.
#[derive(Debug, Error)]
pub enum LlmError {
    /// The provider returned an HTTP error.
    #[error("provider HTTP error {status}: {body}")]
    HttpError {
        /// HTTP status code.
        status: u16,
        /// Response body (may be truncated).
        body: String,
    },

    /// The response could not be parsed.
    #[error("response parse error: {0}")]
    ParseError(String),

    /// Authentication failed or credentials are missing.
    #[error("auth error: {0}")]
    AuthError(String),

    /// The provider does not support the requested model.
    #[error("unsupported model: {0}")]
    UnsupportedModel(String),

    /// The request was rate-limited.
    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds until retry is allowed.
        retry_after_secs: u64,
    },

    /// A streaming connection was interrupted.
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
}

// ── Umbrella error ─────────────────────────────────────────────────────

/// Top-level error that can represent any subsystem failure.
///
/// Use this when a function may fail due to config, execution, IO, or LLM
/// errors and you want a single error type to propagate.
#[derive(Debug, Error)]
pub enum RustcodeError {
    /// Configuration error.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// Command execution error.
    #[error(transparent)]
    Execution(#[from] ExecutionError),

    /// File/process IO error.
    #[error(transparent)]
    Io(#[from] IoError),

    /// LLM provider error.
    #[error(transparent)]
    Llm(#[from] LlmError),

    /// Event publish error.
    #[error(transparent)]
    Publish(#[from] PublishError),

    /// Shutdown error.
    #[error(transparent)]
    Shutdown(#[from] ShutdownError),

    /// Catch-all for other errors.
    #[error("{0}")]
    Other(String),
}

impl From<String> for RustcodeError {
    fn from(msg: String) -> Self {
        Self::Other(msg)
    }
}

impl From<&str> for RustcodeError {
    fn from(msg: &str) -> Self {
        Self::Other(msg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_display() {
        let err = IoError::ReadFailed {
            path: "/tmp/x".to_string(),
            reason: "not found".to_string(),
        };
        assert_eq!(err.to_string(), "file read failed: /tmp/x: not found");
    }

    #[test]
    fn llm_error_display() {
        let err = LlmError::HttpError {
            status: 429,
            body: "too many requests".to_string(),
        };
        assert!(err.to_string().contains("429"));
    }

    #[test]
    fn umbrella_from_config() {
        let config_err = ConfigError::Validation("bad".to_string());
        let err: RustcodeError = config_err.into();
        assert!(matches!(err, RustcodeError::Config(_)));
    }

    #[test]
    fn umbrella_from_string() {
        let err: RustcodeError = "something went wrong".into();
        assert!(err.to_string().contains("something went wrong"));
    }

    #[test]
    fn io_error_from_std() {
        let std_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err: IoError = std_err.into();
        assert!(err.to_string().contains("gone"));
    }
}
