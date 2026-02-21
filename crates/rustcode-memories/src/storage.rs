// ── Memory storage ────────────────────────────────────────────────────
//
// Files are written to:
//   ~/.config/rustcode/memories/raw/{session_id}.md  — per-session extracts
//   ~/.config/rustcode/memories/summary.md           — consolidated summary

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{MemorySummary, RawMemory};

/// Errors from memory storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("memory storage unavailable (HOME not set)")]
    NoHome,

    #[error("failed to create memory directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Manages the on-disk storage of raw memories and the consolidated summary.
#[derive(Debug, Clone)]
pub struct MemoryStorage {
    root: PathBuf,
}

impl MemoryStorage {
    /// Create a new `MemoryStorage` pointing to `~/.config/rustcode/memories/`.
    ///
    /// Returns `None` if the home directory cannot be determined.
    pub fn new() -> Option<Self> {
        let root = memories_dir()?;
        Some(Self { root })
    }

    /// Create a `MemoryStorage` with an explicit root directory (used in tests).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Ensure all required directories exist.
    pub fn init(&self) -> Result<(), StorageError> {
        let raw_dir = self.raw_dir();
        std::fs::create_dir_all(&raw_dir).map_err(|e| StorageError::CreateDir {
            path: raw_dir.clone(),
            source: e,
        })?;
        Ok(())
    }

    /// Save a raw memory for a session.
    pub fn save_raw(&self, memory: &RawMemory) -> Result<(), StorageError> {
        self.init()?;
        let sanitized = sanitize_session_id(&memory.session_id);
        let path = self.raw_dir().join(format!("{sanitized}.md"));
        let body = format!(
            "<!-- session: {} created: {} -->\n{}",
            memory.session_id, memory.created_at, memory.content
        );
        std::fs::write(&path, &body).map_err(|e| StorageError::Write { path, source: e })
    }

    /// Load all raw memory files from the `raw/` directory.
    pub fn load_all_raw(&self) -> Vec<RawMemory> {
        let raw_dir = self.raw_dir();
        let entries = match std::fs::read_dir(&raw_dir) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut memories = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Some(memory) = parse_raw_file(
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown"),
                    &raw,
                ) {
                    memories.push(memory);
                }
            }
        }

        // Sort oldest-first so consolidation is deterministic
        memories.sort_by_key(|m| m.created_at);
        memories
    }

    /// Persist the consolidated memory summary.
    pub fn save_summary(&self, summary: &MemorySummary) -> Result<(), StorageError> {
        self.init()?;
        let path = self.summary_path();
        let body = format!(
            "<!-- updated: {} -->\n{}",
            summary.updated_at, summary.content
        );
        std::fs::write(&path, &body).map_err(|e| StorageError::Write { path, source: e })
    }

    /// Load the consolidated memory summary, if it exists.
    pub fn load_summary(&self) -> Option<MemorySummary> {
        let path = self.summary_path();
        let raw = std::fs::read_to_string(&path).ok()?;
        let content = strip_comment_header(&raw);
        if content.trim().is_empty() {
            return None;
        }

        // Extract timestamp from comment header if present
        let updated_at = parse_header_timestamp(&raw).unwrap_or(0);
        Some(MemorySummary {
            content: content.trim().to_string(),
            updated_at,
        })
    }

    /// Delete all raw memory files (used by `/memory clear`).
    pub fn clear_raw(&self) -> std::io::Result<()> {
        let raw_dir = self.raw_dir();
        if raw_dir.exists() {
            std::fs::remove_dir_all(&raw_dir)?;
            std::fs::create_dir_all(&raw_dir)?;
        }
        Ok(())
    }

    /// Delete the consolidated summary file.
    pub fn clear_summary(&self) -> std::io::Result<()> {
        let path = self.summary_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Delete all raw memories AND the consolidated summary.
    pub fn clear_all(&self) -> std::io::Result<()> {
        self.clear_raw()?;
        self.clear_summary()
    }

    /// Check whether memory collection is disabled.
    ///
    /// Disabled state is indicated by a `disabled` sentinel file in the root.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.root.join("disabled").exists()
    }

    /// Enable or disable memory collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the sentinel file cannot be created or removed.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), StorageError> {
        let sentinel = self.root.join("disabled");
        if enabled {
            if sentinel.exists() {
                std::fs::remove_file(&sentinel).map_err(|e| StorageError::Write {
                    path: sentinel,
                    source: e,
                })?;
            }
        } else {
            self.init()?;
            std::fs::write(&sentinel, b"").map_err(|e| StorageError::Write {
                path: sentinel,
                source: e,
            })?;
        }
        Ok(())
    }

    /// Count the number of raw memory files on disk.
    #[must_use]
    pub fn raw_count(&self) -> usize {
        let raw_dir = self.raw_dir();
        std::fs::read_dir(&raw_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext == "md")
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    fn raw_dir(&self) -> PathBuf {
        self.root.join("raw")
    }

    fn summary_path(&self) -> PathBuf {
        self.root.join("summary.md")
    }

    /// Returns the root memories directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Resolve the memories root directory: `~/.config/rustcode/memories/`
pub fn memories_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("rustcode")
                .join("memories")
        })
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("rustcode").join("memories"))
    }
}

/// Sanitize a session ID for use as a filename.
fn sanitize_session_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

/// Strip the `<!-- ... -->` header comment from a memory file.
fn strip_comment_header(raw: &str) -> String {
    if let Some(end) = raw.find("-->") {
        raw[end + 3..].to_string()
    } else {
        raw.to_string()
    }
}

/// Parse the `updated:` or `created:` timestamp from a comment header.
fn parse_header_timestamp(raw: &str) -> Option<u64> {
    let start = raw.find("updated: ").or_else(|| raw.find("created: "))?;
    let rest = &raw[start..];
    let num_start = rest.find(|c: char| c.is_ascii_digit())?;
    let num_end = rest[num_start..]
        .find(|c: char| !c.is_ascii_digit())
        .map(|e| num_start + e)
        .unwrap_or(rest.len());
    rest[num_start..num_end].parse().ok()
}

/// Parse a raw memory file body back into a `RawMemory`.
fn parse_raw_file(session_id: &str, raw: &str) -> Option<RawMemory> {
    let created_at = parse_header_timestamp(raw).unwrap_or(0);
    let content = strip_comment_header(raw).trim().to_string();
    if content.is_empty() {
        return None;
    }
    Some(RawMemory {
        session_id: session_id.to_string(),
        content,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_storage() -> (tempfile::TempDir, MemoryStorage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = MemoryStorage::with_root(dir.path().to_path_buf());
        (dir, storage)
    }

    #[test]
    fn roundtrip_raw_memory() {
        let (_dir, storage) = tmp_storage();
        let mem = RawMemory::new("ses-abc", "- fact one\n- fact two");
        storage.save_raw(&mem).unwrap();
        let loaded = storage.load_all_raw();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].content.contains("fact one"));
    }

    #[test]
    fn roundtrip_summary() {
        let (_dir, storage) = tmp_storage();
        let summary = MemorySummary::new("User prefers Rust over Python.");
        storage.save_summary(&summary).unwrap();
        let loaded = storage.load_summary().unwrap();
        assert!(loaded.content.contains("Rust"));
    }

    #[test]
    fn no_summary_returns_none() {
        let (_dir, storage) = tmp_storage();
        assert!(storage.load_summary().is_none());
    }

    #[test]
    fn clear_raw_removes_files() {
        let (_dir, storage) = tmp_storage();
        storage.save_raw(&RawMemory::new("ses-1", "data")).unwrap();
        assert!(!storage.load_all_raw().is_empty());
        storage.clear_raw().unwrap();
        assert!(storage.load_all_raw().is_empty());
    }

    #[test]
    fn clear_summary_removes_file() {
        let (_dir, storage) = tmp_storage();
        storage
            .save_summary(&MemorySummary::new("summary text"))
            .unwrap();
        assert!(storage.load_summary().is_some());
        storage.clear_summary().unwrap();
        assert!(storage.load_summary().is_none());
    }

    #[test]
    fn clear_all_removes_raw_and_summary() {
        let (_dir, storage) = tmp_storage();
        storage.save_raw(&RawMemory::new("ses-1", "data")).unwrap();
        storage
            .save_summary(&MemorySummary::new("summary"))
            .unwrap();
        storage.clear_all().unwrap();
        assert!(storage.load_all_raw().is_empty());
        assert!(storage.load_summary().is_none());
    }

    #[test]
    fn enabled_disabled_toggle() {
        let (_dir, storage) = tmp_storage();
        assert!(!storage.is_disabled());
        storage.set_enabled(false).unwrap();
        assert!(storage.is_disabled());
        storage.set_enabled(true).unwrap();
        assert!(!storage.is_disabled());
    }

    #[test]
    fn raw_count_reflects_files() {
        let (_dir, storage) = tmp_storage();
        assert_eq!(storage.raw_count(), 0);
        storage.save_raw(&RawMemory::new("ses-1", "a")).unwrap();
        storage.save_raw(&RawMemory::new("ses-2", "b")).unwrap();
        assert_eq!(storage.raw_count(), 2);
        storage.clear_raw().unwrap();
        assert_eq!(storage.raw_count(), 0);
    }
}
