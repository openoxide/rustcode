use super::*;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use rustcode_core::config::ResolvedConfig;
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::PublishError;
use rustcode_core::event::{Event, EventPayload};
use rustcode_core::permissions::PermissionAction;
use rustcode_core::ports::{EventPublisher, ToolApprover};
use rustcode_core::ToolApprovalRequest;
use rustcode_io::{FileSystemPort, IoError, ProcessOutput, ProcessPort};
use rustcode_llm::{
    ChatRequest, ChatResponse, LlmClient, LlmRequest, LlmResponse, NullLlmClient, TokenUsage,
    ToolCall,
};
use rustcode_plugins::{Plugin, PluginError, PluginRegistry};

mod approvals;
mod core;
mod fixtures;
mod mcp;
mod retry_tests;
mod serve;
mod session_management;
mod tool_validation;
mod webfetch;
