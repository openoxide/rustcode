use std::sync::Arc;
use std::time::SystemTime;

use tokio_util::sync::CancellationToken;

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
    pub cancellation: CancellationToken,
}

impl CommandContext {
    #[must_use]
    pub fn new(config: Arc<ResolvedConfig>, session: SessionMeta) -> Self {
        Self {
            config,
            session,
            cancellation: CancellationToken::new(),
        }
    }

    #[must_use]
    pub fn with_cancellation(
        config: Arc<ResolvedConfig>,
        session: SessionMeta,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            config,
            session,
            cancellation,
        }
    }
}
