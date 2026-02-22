use std::fs;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use rustcode_core::session::{SessionId, SessionInfo, StoredMessage};

use crate::helpers::{
    detect_git_branch, ensure_dir, new_message_id, new_session_id, now_unix_ms, read_json_file,
    set_owner_read_write_only, touch_file, write_json_atomic,
};
use crate::{SessionStore, StateError};

impl SessionStore {
    #[must_use]
    pub fn open_default() -> Self {
        Self {
            root: crate::defaults::default_sessions_root(),
        }
    }

    #[must_use]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create_session(
        &self,
        title: Option<String>,
        parent_id: Option<SessionId>,
        cwd: &Path,
        workspace_root: &Path,
        model: &str,
    ) -> Result<SessionInfo, StateError> {
        if model.trim().is_empty() {
            return Err(StateError::Validation(
                "model must not be empty".to_string(),
            ));
        }

        let id = new_session_id();
        let now = now_unix_ms()?;
        let branch = detect_git_branch(workspace_root)
            .or_else(|| detect_git_branch(cwd))
            .unwrap_or_default();
        let info = SessionInfo {
            id: id.clone(),
            title,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            parent_id,
            cwd: cwd.display().to_string(),
            workspace_root: workspace_root.display().to_string(),
            model: model.to_string(),
            branch,
            total_input_tokens: 0,
            total_output_tokens: 0,
            cost_usd: 0.0,
        };

        let dir = self.session_dir(&id);
        ensure_dir(&dir)?;
        write_json_atomic(&dir.join("meta.json"), &info)?;
        touch_file(&dir.join("messages.jsonl"))?;

        Ok(info)
    }

    pub fn get_session(&self, session_id: &str) -> Result<SessionInfo, StateError> {
        if session_id.trim().is_empty() {
            return Err(StateError::Validation(
                "session id must not be empty".to_string(),
            ));
        }
        let path = self.session_dir(session_id).join("meta.json");
        if !path.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        read_json_file(&path)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, StateError> {
        let mut sessions = Vec::new();
        if !self.root.exists() {
            return Ok(sessions);
        }

        let entries = fs::read_dir(&self.root)
            .map_err(|err| StateError::Io(format!("{}: {err}", self.root.display())))?;
        for entry in entries {
            let entry = entry.map_err(|err| StateError::Io(err.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join("meta.json");
            if !meta_path.exists() {
                continue;
            }
            if let Ok(info) = read_json_file::<SessionInfo>(&meta_path) {
                sessions.push(info);
            }
        }

        sessions.sort_by(|a, b| b.updated_at_unix_ms.cmp(&a.updated_at_unix_ms));
        Ok(sessions)
    }

    pub fn append_message(
        &self,
        session_id: &str,
        message: &StoredMessage,
    ) -> Result<(), StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let messages_path = dir.join("messages.jsonl");

        let mut info = self.get_session(session_id)?;
        info.updated_at_unix_ms = now_unix_ms()?;
        write_json_atomic(&dir.join("meta.json"), &info)?;

        let line = serde_json::to_string(message)
            .map_err(|err| StateError::Json(format!("failed to serialize message: {err}")))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&messages_path)
            .map_err(|err| StateError::Io(format!("{}: {err}", messages_path.display())))?;
        file.write_all(line.as_bytes())
            .map_err(|err| StateError::Io(err.to_string()))?;
        file.write_all(b"\n")
            .map_err(|err| StateError::Io(err.to_string()))?;
        file.flush()
            .map_err(|err| StateError::Io(err.to_string()))?;

        set_owner_read_write_only(&messages_path)?;
        Ok(())
    }

    pub fn clear_messages(&self, session_id: &str) -> Result<(), StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let messages_path = dir.join("messages.jsonl");
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&messages_path)
            .map_err(|err| StateError::Io(format!("{}: {err}", messages_path.display())))?;
        set_owner_read_write_only(&messages_path)?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let messages_path = dir.join("messages.jsonl");
        if !messages_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&messages_path)
            .map_err(|err| StateError::Io(format!("{}: {err}", messages_path.display())))?;
        let reader = BufReader::new(file);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|err| StateError::Io(err.to_string()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg = serde_json::from_str::<StoredMessage>(trimmed).map_err(|err| {
                StateError::Json(format!(
                    "{}: failed to parse message json: {err}",
                    messages_path.display()
                ))
            })?;
            out.push(msg);
        }
        Ok(out)
    }

    pub fn fork_session(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<SessionInfo, StateError> {
        let src = self.get_session(session_id)?;
        let src_dir = self.session_dir(session_id);

        let forked = self.create_session(
            title.or_else(|| src.title.clone()),
            Some(src.id.clone()),
            Path::new(&src.cwd),
            Path::new(&src.workspace_root),
            &src.model,
        )?;

        let dst_dir = self.session_dir(&forked.id);
        let src_messages = src_dir.join("messages.jsonl");
        let dst_messages = dst_dir.join("messages.jsonl");
        if src_messages.exists() {
            fs::copy(&src_messages, &dst_messages).map_err(|err| {
                StateError::Io(format!(
                    "failed to copy {} to {}: {err}",
                    src_messages.display(),
                    dst_messages.display()
                ))
            })?;
            set_owner_read_write_only(&dst_messages)?;
        }

        Ok(forked)
    }

    pub fn update_session_title(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<SessionInfo, StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let mut info = self.get_session(session_id)?;
        info.title = title.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        info.updated_at_unix_ms = now_unix_ms()?;
        write_json_atomic(&dir.join("meta.json"), &info)?;
        Ok(info)
    }

    pub fn update_session_model(
        &self,
        session_id: &str,
        model: &str,
    ) -> Result<SessionInfo, StateError> {
        let next_model = model.trim();
        if next_model.is_empty() {
            return Err(StateError::Validation(
                "model must not be empty".to_string(),
            ));
        }
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let mut info = self.get_session(session_id)?;
        info.model = next_model.to_string();
        info.updated_at_unix_ms = now_unix_ms()?;
        write_json_atomic(&dir.join("meta.json"), &info)?;
        Ok(info)
    }

    pub fn update_session_usage(
        &self,
        session_id: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
        cost_usd: f64,
    ) -> Result<SessionInfo, StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        let mut info = self.get_session(session_id)?;
        info.total_input_tokens = total_input_tokens;
        info.total_output_tokens = total_output_tokens;
        info.cost_usd = cost_usd;
        info.updated_at_unix_ms = now_unix_ms()?;
        write_json_atomic(&dir.join("meta.json"), &info)?;
        Ok(info)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), StateError> {
        let dir = self.session_dir(session_id);
        if !dir.exists() {
            return Err(StateError::NotFound(session_id.to_string()));
        }
        fs::remove_dir_all(&dir)
            .map_err(|err| StateError::Io(format!("{}: {err}", dir.display())))?;
        Ok(())
    }

    #[must_use]
    pub fn new_message_id(&self) -> rustcode_core::session::MessageId {
        new_message_id()
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }
}
