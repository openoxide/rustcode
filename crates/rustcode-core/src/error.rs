use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file read failed: {0}")]
    Read(String),
    #[error("config parse failed: {0}")]
    Parse(String),
    #[error("config validation failed: {0}")]
    Validation(String),
    #[error("config trust check failed: {0}")]
    Trust(String),
}

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("command dispatch failed: {0}")]
    Dispatch(String),
    #[error("executor failed: {0}")]
    Executor(String),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("event sink closed")]
    SinkClosed,
}

#[derive(Debug, Error)]
pub enum ShutdownError {
    #[error("shutdown timed out")]
    Timeout,
}
