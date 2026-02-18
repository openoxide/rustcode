use std::path::PathBuf;

use rustcode_core::session::{SessionInfo, StoredMessage};
use rustcode_state::SessionStore;

#[derive(Debug, Clone)]
pub struct CreateSessionOptions {
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub model: String,
}

pub trait SessionBackend: Send + Sync {
    fn list_sessions(&self) -> Result<Vec<SessionInfo>, String>;
    fn get_session(&self, session_id: &str) -> Result<SessionInfo, String>;
    fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, String>;
    fn create_session(&self, options: CreateSessionOptions) -> Result<SessionInfo, String>;
    fn fork_session(&self, session_id: &str, title: Option<String>) -> Result<SessionInfo, String>;
}

#[derive(Debug, Clone)]
pub struct LocalSessionBackend {
    store: SessionStore,
}

impl LocalSessionBackend {
    #[must_use]
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }
}

impl SessionBackend for LocalSessionBackend {
    fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        self.store.list_sessions().map_err(|err| err.to_string())
    }

    fn get_session(&self, session_id: &str) -> Result<SessionInfo, String> {
        self.store
            .get_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, String> {
        self.store
            .load_messages(session_id)
            .map_err(|err| err.to_string())
    }

    fn create_session(&self, options: CreateSessionOptions) -> Result<SessionInfo, String> {
        self.store
            .create_session(
                options.title,
                options.parent_id,
                &options.cwd,
                &options.workspace_root,
                &options.model,
            )
            .map_err(|err| err.to_string())
    }

    fn fork_session(&self, session_id: &str, title: Option<String>) -> Result<SessionInfo, String> {
        self.store
            .fork_session(session_id, title)
            .map_err(|err| err.to_string())
    }
}
