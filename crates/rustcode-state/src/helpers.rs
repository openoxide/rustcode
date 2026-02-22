use std::fmt::Write as FmtWrite;
use std::fs;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;

use rustcode_core::session::{MessageId, SessionId};

use crate::StateError;

pub(crate) fn ensure_dir(path: &Path) -> Result<(), StateError> {
    fs::create_dir_all(path).map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    set_owner_read_write_exec_only(path)?;
    Ok(())
}

pub(crate) fn touch_file(path: &Path) -> Result<(), StateError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    set_owner_read_write_only(path)?;
    Ok(())
}

pub(crate) fn write_json_atomic<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), StateError> {
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

pub(crate) fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, StateError> {
    let raw = fs::read_to_string(path)
        .map_err(|err| StateError::Io(format!("{}: {err}", path.display())))?;
    serde_json::from_str(&raw).map_err(|err| StateError::Json(format!("{}: {err}", path.display())))
}

pub(crate) fn now_unix_ms() -> Result<i64, StateError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| StateError::Io(format!("system clock error: {err}")))?;
    i64::try_from(now.as_millis()).map_err(|err| StateError::Io(err.to_string()))
}

pub(crate) fn new_session_id() -> SessionId {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 7];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(10);
    out.push_str("rc-");
    for b in bytes {
        out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
    }
    out
}

pub(crate) fn new_message_id() -> MessageId {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let mut rand_hex = String::with_capacity(16);
    for b in &bytes {
        let _ = write!(rand_hex, "{b:02x}");
    }
    format!("m-{now}-{rand_hex}")
}

pub(crate) fn detect_git_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

#[cfg(unix)]
pub(crate) fn set_owner_read_write_only(path: &Path) -> Result<(), StateError> {
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
pub(crate) fn set_owner_read_write_only(_path: &Path) -> Result<(), StateError> {
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
