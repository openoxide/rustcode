use std::path::PathBuf;
use std::time::Duration;

use rustcode_core::server_protocol::{
    V1SessionCreateRequest, V1SessionCreateResponse, V1SessionShowResponse, V1SessionsListResponse,
    SERVER_API_SCHEMA_VERSION,
};
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

    fn update_session_title(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<SessionInfo, String>;

    fn update_session_model(&self, session_id: &str, model: String) -> Result<SessionInfo, String>;

    /// Persist cumulative token usage and cost to the session metadata.
    ///
    /// Called when a run completes so the data survives across restarts.
    /// Remote backends silently ignore this (no-op).
    fn update_session_usage(
        &self,
        session_id: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
        cost_usd: f64,
    ) -> Result<(), String>;

    fn delete_session(&self, session_id: &str) -> Result<(), String>;

    /// Clear all messages from a session's transcript without deleting the session.
    fn clear_messages(&self, session_id: &str) -> Result<(), String>;
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

    fn update_session_title(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<SessionInfo, String> {
        self.store
            .update_session_title(session_id, title)
            .map_err(|err| err.to_string())
    }

    fn update_session_model(&self, session_id: &str, model: String) -> Result<SessionInfo, String> {
        self.store
            .update_session_model(session_id, &model)
            .map_err(|err| err.to_string())
    }

    fn update_session_usage(
        &self,
        session_id: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
        cost_usd: f64,
    ) -> Result<(), String> {
        self.store
            .update_session_usage(
                session_id,
                total_input_tokens,
                total_output_tokens,
                cost_usd,
            )
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.store
            .delete_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn clear_messages(&self, session_id: &str) -> Result<(), String> {
        self.store
            .clear_messages(session_id)
            .map_err(|err| err.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RemoteSessionBackend {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl RemoteSessionBackend {
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        let normalized = base_url.trim_end_matches('/').to_string();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            base_url: normalized,
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    fn response_error(prefix: &str, status: reqwest::StatusCode, body: String) -> String {
        if body.trim().is_empty() {
            format!("{prefix} failed ({status})")
        } else {
            format!("{prefix} failed ({status}): {body}")
        }
    }

    fn fetch_show(&self, session_id: &str) -> Result<V1SessionShowResponse, String> {
        let endpoint = self.url(&format!("/v1/sessions/{session_id}"));
        let response = self
            .client
            .get(endpoint)
            .send()
            .map_err(|err| format!("show session request failed: {err}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(Self::response_error("show session", status, body));
        }
        response
            .json::<V1SessionShowResponse>()
            .map_err(|err| format!("show session decode failed: {err}"))
    }
}

impl SessionBackend for RemoteSessionBackend {
    fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        let endpoint = self.url("/v1/sessions");
        let response = self
            .client
            .get(endpoint)
            .send()
            .map_err(|err| format!("list sessions request failed: {err}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(Self::response_error("list sessions", status, body));
        }
        let payload = response
            .json::<V1SessionsListResponse>()
            .map_err(|err| format!("list sessions decode failed: {err}"))?;
        Ok(payload.sessions)
    }

    fn get_session(&self, session_id: &str) -> Result<SessionInfo, String> {
        let show = self.fetch_show(session_id)?;
        Ok(show.session)
    }

    fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, String> {
        let show = self.fetch_show(session_id)?;
        Ok(show.messages)
    }

    fn create_session(&self, options: CreateSessionOptions) -> Result<SessionInfo, String> {
        let endpoint = self.url("/v1/sessions");
        let request = V1SessionCreateRequest {
            schema_version: SERVER_API_SCHEMA_VERSION,
            title: options.title,
        };

        let response = self
            .client
            .post(endpoint)
            .json(&request)
            .send()
            .map_err(|err| format!("create session request failed: {err}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(Self::response_error("create session", status, body));
        }
        let payload = response
            .json::<V1SessionCreateResponse>()
            .map_err(|err| format!("create session decode failed: {err}"))?;
        Ok(payload.session)
    }

    fn fork_session(
        &self,
        _session_id: &str,
        _title: Option<String>,
    ) -> Result<SessionInfo, String> {
        Err("fork is not supported in tui attach mode".to_string())
    }

    fn update_session_title(
        &self,
        _session_id: &str,
        _title: Option<String>,
    ) -> Result<SessionInfo, String> {
        Err("rename is not supported in tui attach mode".to_string())
    }

    fn update_session_model(
        &self,
        _session_id: &str,
        _model: String,
    ) -> Result<SessionInfo, String> {
        Err("model update is not supported in tui attach mode".to_string())
    }

    fn update_session_usage(
        &self,
        _session_id: &str,
        _total_input_tokens: u64,
        _total_output_tokens: u64,
        _cost_usd: f64,
    ) -> Result<(), String> {
        Ok(()) // no-op in attach mode — server owns the session metadata
    }

    fn delete_session(&self, _session_id: &str) -> Result<(), String> {
        Err("delete is not supported in tui attach mode".to_string())
    }

    fn clear_messages(&self, _session_id: &str) -> Result<(), String> {
        Err("clear is not supported in tui attach mode".to_string())
    }
}
