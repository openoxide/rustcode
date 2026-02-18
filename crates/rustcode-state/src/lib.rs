use std::fs;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use thiserror::Error;

use rustcode_core::session::{MessageId, SessionId, SessionInfo, StoredMessage};
use rustcode_core::{ExecutionError, TranscriptRecorder};

use async_trait::async_trait;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("session not found: {0}")]
    NotFound(SessionId),
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    #[must_use]
    pub fn open_default() -> Self {
        Self {
            root: default_sessions_root(),
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
        let info = SessionInfo {
            id: id.clone(),
            title,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            parent_id,
            cwd: cwd.display().to_string(),
            workspace_root: workspace_root.display().to_string(),
            model: model.to_string(),
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
            match read_json_file::<SessionInfo>(&meta_path) {
                Ok(info) => sessions.push(info),
                Err(_) => {
                    // Skip unreadable sessions; caller can inspect on disk.
                    continue;
                }
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

    pub fn new_message_id(&self) -> MessageId {
        new_message_id()
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }
}

#[derive(Debug, Clone)]
pub struct FileTranscriptRecorder {
    store: SessionStore,
}

impl FileTranscriptRecorder {
    #[must_use]
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl TranscriptRecorder for FileTranscriptRecorder {
    async fn append_message(
        &self,
        session_id: &str,
        message: StoredMessage,
    ) -> Result<(), ExecutionError> {
        let store = self.store.clone();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || store.append_message(&session_id, &message))
            .await
            .map_err(|err| {
                ExecutionError::Executor(format!("transcript recorder join error: {err}"))
            })?
            .map_err(|err| ExecutionError::Executor(err.to_string()))
    }
}

fn default_sessions_root() -> PathBuf {
    if let Ok(path) = std::env::var("RUSTCODE_SESSIONS_DIR") {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("rustcode/sessions");
    }
    if let Ok(path) = std::env::var("HOME") {
        return PathBuf::from(path).join(".local/share/rustcode/sessions");
    }
    PathBuf::from(".rustcode-sessions")
}

fn ensure_dir(path: &Path) -> Result<(), StateError> {
    fs::create_dir_all(path).map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    set_owner_read_write_exec_only(path)?;
    Ok(())
}

fn touch_file(path: &Path) -> Result<(), StateError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    set_owner_read_write_only(path)?;
    Ok(())
}

fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    let serialized = serde_json::to_string_pretty(value)
        .map_err(|err| StateError::Json(format!("failed to serialize json: {err}")))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serialized)
        .map_err(|err| StateError::Io(format!("{}: {err}", temp.display())))?;
    fs::rename(&temp, path).map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    set_owner_read_write_only(path)?;
    Ok(())
}

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, StateError> {
    let raw = fs::read_to_string(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    serde_json::from_str(&raw).map_err(|err| StateError::Json(format!("{}: {err}", path.display())))
}

fn now_unix_ms() -> Result<i64, StateError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| StateError::Io(format!("system clock error: {err}")))?;
    i64::try_from(now.as_millis()).map_err(|err| StateError::Io(err.to_string()))
}

fn new_session_id() -> SessionId {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let rand_hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("s-{now}-{rand_hex}")
}

fn new_message_id() -> MessageId {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let rand_hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("m-{now}-{rand_hex}")
}

#[cfg(unix)]
fn set_owner_read_write_only(path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_read_write_only(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_read_write_exec_only(path: &Path) -> Result<(), StateError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_read_write_exec_only(_path: &Path) -> Result<(), StateError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rustcode_core::session::{MessageRole, StoredToolCall};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time must be monotonic")
            .as_nanos();
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("rustcode-state-{name}-{pid}-{now}"));
        fs::create_dir_all(&dir).expect("must create temp dir");
        dir
    }

    #[test]
    fn create_list_append_and_load_round_trip() {
        let root = temp_dir("roundtrip");
        let store = SessionStore::with_root(root.join("sessions"));

        let session = store
            .create_session(
                Some("t1".to_string()),
                None,
                Path::new("/tmp"),
                Path::new("/tmp"),
                "openai/gpt-test",
            )
            .expect("create session");

        let sessions = store.list_sessions().expect("list sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session.id);

        let msg = StoredMessage {
            id: store.new_message_id(),
            role: MessageRole::User,
            created_at_unix_ms: now_unix_ms().expect("now"),
            content: serde_json::Value::String("hello".to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: vec![StoredToolCall {
                id: "call_1".to_string(),
                name: "list".to_string(),
                arguments: "{}".to_string(),
            }],
        };
        store
            .append_message(&session.id, &msg)
            .expect("append message");

        let loaded = store.load_messages(&session.id).expect("load messages");
        assert_eq!(loaded, vec![msg]);
    }

    #[test]
    fn fork_clones_messages_and_sets_parent() {
        let root = temp_dir("fork");
        let store = SessionStore::with_root(root.join("sessions"));

        let session = store
            .create_session(None, None, Path::new("/tmp"), Path::new("/tmp"), "m")
            .expect("create session");

        let msg = StoredMessage {
            id: store.new_message_id(),
            role: MessageRole::Assistant,
            created_at_unix_ms: now_unix_ms().expect("now"),
            content: serde_json::Value::String("hi".to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        };
        store
            .append_message(&session.id, &msg)
            .expect("append message");

        let forked = store
            .fork_session(&session.id, Some("forked".to_string()))
            .expect("fork");
        assert_eq!(forked.parent_id.as_deref(), Some(session.id.as_str()));
        assert_eq!(forked.title.as_deref(), Some("forked"));

        let loaded = store.load_messages(&forked.id).expect("load");
        assert_eq!(loaded, vec![msg]);
    }

    #[test]
    fn load_unknown_session_is_not_found() {
        let root = temp_dir("not-found");
        let store = SessionStore::with_root(root.join("sessions"));
        let err = store
            .load_messages("does-not-exist")
            .expect_err("must fail");
        assert!(matches!(err, StateError::NotFound(_)));
    }
}
