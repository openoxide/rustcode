use thiserror::Error;

/// Errors during configuration loading, parsing, or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read a configuration file from disk.
    #[error("config file read failed: {0}")]
    Read(String),
    /// Configuration file contents could not be parsed.
    #[error("config parse failed: {0}")]
    Parse(String),
    /// Configuration values failed validation checks.
    #[error("config validation failed: {0}")]
    Validation(String),
    /// Project-level config was not trusted by the user.
    #[error("config trust check failed: {0}")]
    Trust(String),
}

/// Errors during command or tool execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// The tool call was dispatched but arguments were invalid.
    #[error("command dispatch failed: {0}")]
    Dispatch(String),
    /// The tool's underlying operation failed.
    #[error("executor failed: {0}")]
    Executor(String),
    /// The operation was cancelled (e.g., by the user or timeout).
    #[error("cancelled")]
    Cancelled,
}

/// Errors when publishing events to subscribers.
#[derive(Debug, Error)]
pub enum PublishError {
    /// The event sink (channel) was closed before the event could be sent.
    #[error("event sink closed")]
    SinkClosed,
}

/// Errors during graceful shutdown.
#[derive(Debug, Error)]
pub enum ShutdownError {
    /// The shutdown process did not complete within the allotted time.
    #[error("shutdown timed out")]
    Timeout,
}
