use crate::cli::SessionCommand;
use crate::utils::write_stdout_line;
use anyhow::{Context, Result};
use rustcode_core::session::{SessionInfo, StoredMessage};
use rustcode_state::SessionStore;

pub fn handle_session_command(command: SessionCommand, json_output: bool) -> Result<()> {
    let store = SessionStore::open_default();
    match command {
        SessionCommand::List => {
            let sessions = store.list_sessions()?;
            if json_output {
                let mut rows = Vec::with_capacity(sessions.len());
                for meta in sessions {
                    rows.push(serde_json::json!({
                        "id": meta.id,
                        "title": meta.title,
                        "created_at": meta.created_at_unix_ms,
                    }));
                }
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.list",
                    "sessions": rows,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("sessions={}", sessions.len()))?;
                for meta in sessions {
                    write_stdout_line(&format!(
                        "id={}\ttitle={}\tcreated_at={}\tupdated_at={}",
                        meta.id,
                        meta.title.as_deref().unwrap_or(""),
                        meta.created_at_unix_ms,
                        meta.updated_at_unix_ms
                    ))?;
                }
            }
        }
        SessionCommand::Show { session_id: id } => {
            let meta = store.get_session(&id)?;
            let messages = store.load_messages(&id)?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.show",
                    "meta": meta,
                    "messages": messages,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("id={}", meta.id))?;
                write_stdout_line(&format!("title={}", meta.title.unwrap_or_default()))?;
                write_stdout_line(&format!("parent_id={}", meta.parent_id.unwrap_or_default()))?;
                write_stdout_line(&format!("messages={}", messages.len()))?;
            }
        }
        SessionCommand::New { title } => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let meta = store.create_session(title, None, &cwd, &cwd, "default")?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.new",
                    "id": meta.id,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("id={}", meta.id))?;
            }
        }
        SessionCommand::Fork {
            session_id: id,
            title,
        } => {
            let meta = store.fork_session(&id, title)?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.fork",
                    "id": meta.id,
                    "parent": id,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("id={}", meta.id))?;
            }
        }
    }
    Ok(())
}

pub fn handle_export_command(session_id: Option<&str>) -> Result<()> {
    let store = SessionStore::open_default();
    let id = if let Some(id) = session_id {
        id.to_string()
    } else {
        let sessions = store.list_sessions()?;
        sessions
            .into_iter()
            .next()
            .context("no sessions found to export")?
            .id
    };

    let meta = store.get_session(&id)?;
    let messages = store.load_messages(&id)?;
    let payload = serde_json::json!({
        "schema_version": 1,
        "session": meta,
        "messages": messages,
    });
    write_stdout_line(&serde_json::to_string_pretty(&payload)?)?;
    Ok(())
}

pub fn handle_import_command(file: &str) -> Result<()> {
    let content =
        std::fs::read_to_string(file).context(format!("failed to read export file {file}"))?;
    let payload: serde_json::Value =
        serde_json::from_str(&content).context("failed to parse export json")?;

    let store = SessionStore::open_default();
    let session_value = if payload.get("session").is_some() {
        payload["session"].clone()
    } else {
        payload["meta"].clone()
    };
    let meta: SessionInfo = serde_json::from_value(session_value)
        .context("failed to deserialize session meta from export")?;
    let messages: Vec<StoredMessage> = serde_json::from_value(payload["messages"].clone())
        .context("failed to deserialize messages from export")?;

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let new_session = store.create_session(meta.title, None, &cwd, &cwd, &meta.model)?;
    for message in messages {
        store.append_message(&new_session.id, &message)?;
    }

    write_stdout_line(&format!("Imported session: {}", new_session.id))?;
    Ok(())
}

pub fn resolve_session(
    store: &SessionStore,
    continue_session: bool,
    session_id: Option<String>,
    fork_session: Option<String>,
    title: Option<String>,
    model: &str,
) -> Result<String> {
    if let Some(id) = session_id {
        return Ok(id);
    }

    if let Some(id) = fork_session {
        let meta = store.fork_session(&id, title)?;
        return Ok(meta.id);
    }

    if continue_session {
        let sessions = store.list_sessions()?;
        if let Some(meta) = sessions.into_iter().next() {
            return Ok(meta.id);
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let meta = store.create_session(title, None, &cwd, &cwd, model)?;
    Ok(meta.id)
}
