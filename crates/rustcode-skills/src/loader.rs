// ── Skill file loader ─────────────────────────────────────────────────
//
// Scans skill directories for `*.md` files with TOML frontmatter and
// returns parsed `SkillFile` instances.  Invalid files are logged and
// skipped rather than causing a hard failure.
//
// Frontmatter format:
//
//   ---
//   name = "code-review"
//   description = "Thorough code review"
//   enabled = true
//   ---
//   Skill content (markdown) ...

use std::path::{Path, PathBuf};

use tracing::warn;

use crate::model::{SkillError, SkillFile, SkillMetadata};

const FRONTMATTER_DELIMITER: &str = "---";

/// Parse a single skill `.md` file.
///
/// Returns `Ok(SkillFile)` on success, or a `SkillError` describing what went wrong.
pub fn parse_skill_file(path: &Path) -> Result<SkillFile, SkillError> {
    let raw = std::fs::read_to_string(path).map_err(|_| SkillError::MissingFrontmatter {
        path: path.to_owned(),
    })?;

    // Must start with `---` (optionally with trailing whitespace)
    let first_line_end = raw.find('\n').unwrap_or(raw.len());
    let first_line = raw[..first_line_end].trim();
    if first_line != FRONTMATTER_DELIMITER {
        return Err(SkillError::MissingFrontmatter {
            path: path.to_owned(),
        });
    }

    // Find the closing `---`
    let after_open = &raw[first_line_end + 1..];
    let close_pos = after_open
        .lines()
        .enumerate()
        .find(|(_, line)| line.trim() == FRONTMATTER_DELIMITER)
        .map(|(idx, _)| idx);

    let Some(close_line_idx) = close_pos else {
        return Err(SkillError::UnclosedFrontmatter {
            path: path.to_owned(),
        });
    };

    // Reconstruct byte offset of the closing delimiter
    let toml_str: String = after_open
        .lines()
        .take(close_line_idx)
        .collect::<Vec<_>>()
        .join("\n");

    // Content is everything after the closing `---` line
    let content_start_line = close_line_idx + 1; // skip the `---` line itself
    let content: String = after_open
        .lines()
        .skip(content_start_line)
        .collect::<Vec<_>>()
        .join("\n");

    let metadata: SkillMetadata =
        toml::from_str(&toml_str).map_err(|e| SkillError::InvalidToml {
            path: path.to_owned(),
            source: e,
        })?;

    if metadata.name.trim().is_empty() {
        return Err(SkillError::EmptyName {
            path: path.to_owned(),
        });
    }

    Ok(SkillFile {
        metadata,
        content: content.trim().to_string(),
        path: path.to_owned(),
    })
}

/// Scan a directory for `*.md` skill files.
///
/// Files that fail to parse are logged at `WARN` level and skipped.
/// Returns a list of successfully parsed skills.
pub fn scan_dir(dir: &Path) -> Vec<SkillFile> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match parse_skill_file(&path) {
            Ok(skill) => skills.push(skill),
            Err(e) => warn!("skipping invalid skill file: {e}"),
        }
    }

    // Sort deterministically by name
    skills.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
    skills
}

/// Resolve the global skills directory: `~/.config/rustcode/skills/`
pub fn global_skills_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("rustcode")
                .join("skills")
        })
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("rustcode").join("skills"))
    }
}

/// Resolve the project-level skills directory: `{workspace}/.rustcode/skills/`
pub fn project_skills_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".rustcode").join("skills")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_tmp(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skill.md");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        (dir, path)
    }

    #[test]
    fn parses_valid_skill() {
        let src = "---\nname = \"test\"\ndescription = \"A test skill\"\n---\nDo the thing.\n";
        let (_dir, path) = write_tmp(src);
        let skill = parse_skill_file(&path).unwrap();
        assert_eq!(skill.name(), "test");
        assert_eq!(skill.description(), "A test skill");
        assert!(skill.metadata.enabled);
        assert_eq!(skill.content, "Do the thing.");
    }

    #[test]
    fn parses_disabled_skill() {
        let src = "---\nname = \"off\"\ndescription = \"\"\nenabled = false\n---\nContent.\n";
        let (_dir, path) = write_tmp(src);
        let skill = parse_skill_file(&path).unwrap();
        assert!(!skill.metadata.enabled);
    }

    #[test]
    fn error_on_missing_frontmatter() {
        let src = "Just plain markdown\n";
        let (_dir, path) = write_tmp(src);
        assert!(parse_skill_file(&path).is_err());
    }

    #[test]
    fn error_on_unclosed_frontmatter() {
        let src = "---\nname = \"x\"\n";
        let (_dir, path) = write_tmp(src);
        assert!(parse_skill_file(&path).is_err());
    }

    #[test]
    fn error_on_empty_name() {
        let src = "---\nname = \"\"\n---\nContent.\n";
        let (_dir, path) = write_tmp(src);
        assert!(parse_skill_file(&path).is_err());
    }

    #[test]
    fn scan_dir_returns_sorted_skills() {
        let dir = tempfile::tempdir().unwrap();
        for (name, fname) in [("bravo", "b.md"), ("alpha", "a.md")] {
            let src = format!("---\nname = \"{name}\"\ndescription = \"\"\n---\nContent.\n");
            std::fs::write(dir.path().join(fname), src).unwrap();
        }
        let skills = scan_dir(dir.path());
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name(), "alpha");
        assert_eq!(skills[1].name(), "bravo");
    }

    #[test]
    fn scan_dir_missing_returns_empty() {
        let skills = scan_dir(std::path::Path::new("/nonexistent/path/skills"));
        assert!(skills.is_empty());
    }
}
