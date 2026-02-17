use std::sync::Arc;
use std::time::SystemTime;

use crate::config::ResolvedConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub session_id: String,
    pub request_id: String,
    pub started_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct CommandContext {
    pub config: Arc<ResolvedConfig>,
    pub session: SessionMeta,
}

impl CommandContext {
    pub fn new(config: Arc<ResolvedConfig>, session: SessionMeta) -> Self {
        Self { config, session }
    }
}
