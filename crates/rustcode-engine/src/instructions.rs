// ── Instruction file loader ───────────────────────────────────────────
//
// Discovers and loads workspace/global instruction files (RUSTCODE.md,
// AGENTS.md) and formats them for injection into the system prompt.
//
// Search order (first match per location wins):
//   1. Workspace root:  RUSTCODE.md → AGENTS.md
//   2. Global config:   ~/.config/rustcode/AGENTS.md

use std::path::{Path, PathBuf};

/// Well-known instruction file names, checked in priority order.
const INSTRUCTION_FILES: &[&str] = &["RUSTCODE.md", "AGENTS.md"];

/// Represents a loaded instruction file.
#[derive(Debug, Clone)]
pub struct InstructionFile {
    pub path: PathBuf,
    pub content: String,
}

/// Find and load the first matching instruction file in `dir`.
///
/// Returns `None` if no instruction file exists in the directory.
fn find_instruction_in_dir(dir: &Path) -> Option<InstructionFile> {
    for name in INSTRUCTION_FILES {
        let path = dir.join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(InstructionFile {
                    path,
                    content: trimmed,
                });
            }
        }
    }
    None
}

/// Default global config directory for rustcode.
///
/// Returns `~/.config/rustcode` on Unix-like or `%APPDATA%/rustcode` on Windows.
fn global_config_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("rustcode"))
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("rustcode"))
    }
}

/// Load all instruction files from workspace root and global config.
///
/// Returns a list of instruction files found (workspace first, then global).
/// At most one file per location is returned (the first match by priority).
pub fn load_instructions(workspace_root: &Path) -> Vec<InstructionFile> {
    let mut results = Vec::new();

    // 1. Workspace root
    if let Some(file) = find_instruction_in_dir(workspace_root) {
        results.push(file);
    }

    // 2. Walk up parent directories to find project-level instructions
    // (stops at workspace root, so this checks subdirectory overrides)
    // -- skip, workspace root is already covered

    // 3. Global config directory (~/.config/rustcode/)
    if let Some(config_dir) = global_config_dir() {
        if config_dir != workspace_root {
            if let Some(file) = find_instruction_in_dir(&config_dir) {
                // Avoid duplicate if global config is same file
                let dominated = results.iter().any(|r| r.path == file.path);
                if !dominated {
                    results.push(file);
                }
            }
        }
    }

    results
}

/// Format loaded instruction files into text suitable for the system prompt.
///
/// Each file is rendered as:
/// ```text
/// <instructions source="path/to/file">
/// file contents...
/// </instructions>
/// ```
pub fn format_instructions(files: &[InstructionFile]) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for file in files {
        out.push_str(&format!(
            "<instructions source=\"{}\">\n{}\n</instructions>\n\n",
            file.path.display(),
            file.content,
        ));
    }
    out.trim_end().to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rustcode-instr-test")
            .join(name)
            .join(format!("{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn find_instruction_prefers_rustcode_md() {
        let dir = test_dir("prefer-rustcode");
        fs::write(dir.join("RUSTCODE.md"), "rustcode instructions").unwrap();
        fs::write(dir.join("AGENTS.md"), "agents instructions").unwrap();

        let file = find_instruction_in_dir(&dir).expect("should find file");
        assert!(file.path.ends_with("RUSTCODE.md"));
        assert_eq!(file.content, "rustcode instructions");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_instruction_falls_back_to_agents_md() {
        let dir = test_dir("fallback-agents");
        fs::write(dir.join("AGENTS.md"), "agents instructions").unwrap();

        let file = find_instruction_in_dir(&dir).expect("should find file");
        assert!(file.path.ends_with("AGENTS.md"));
        assert_eq!(file.content, "agents instructions");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_instruction_skips_empty_files() {
        let dir = test_dir("skip-empty");
        fs::write(dir.join("RUSTCODE.md"), "  \n  ").unwrap();
        fs::write(dir.join("AGENTS.md"), "real content").unwrap();

        let file = find_instruction_in_dir(&dir).expect("should find file");
        assert!(file.path.ends_with("AGENTS.md"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_instruction_returns_none_when_missing() {
        let dir = test_dir("none-missing");
        assert!(find_instruction_in_dir(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_instructions_collects_workspace_files() {
        let dir = test_dir("load-workspace");
        fs::write(dir.join("AGENTS.md"), "workspace agent instructions").unwrap();

        let files = load_instructions(&dir);
        assert!(!files.is_empty());
        assert!(files[0].content.contains("workspace agent"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_instructions_wraps_in_tags() {
        let files = vec![InstructionFile {
            path: PathBuf::from("/project/RUSTCODE.md"),
            content: "Do the thing.".to_string(),
        }];
        let formatted = format_instructions(&files);
        assert!(formatted.contains("<instructions source=\"/project/RUSTCODE.md\">"));
        assert!(formatted.contains("Do the thing."));
        assert!(formatted.contains("</instructions>"));
    }

    #[test]
    fn format_instructions_empty_returns_empty() {
        assert_eq!(format_instructions(&[]), "");
    }

    #[test]
    fn format_instructions_multiple_files() {
        let files = vec![
            InstructionFile {
                path: PathBuf::from("/a/RUSTCODE.md"),
                content: "First".to_string(),
            },
            InstructionFile {
                path: PathBuf::from("/b/AGENTS.md"),
                content: "Second".to_string(),
            },
        ];
        let formatted = format_instructions(&files);
        assert!(formatted.contains("First"));
        assert!(formatted.contains("Second"));
        let pos_first = formatted.find("First").unwrap();
        let pos_second = formatted.find("Second").unwrap();
        assert!(pos_first < pos_second);
    }
}
