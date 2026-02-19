//! Skill file discovery, parsing, and management for rustcode.
//!
//! Skills are user-authored markdown files with TOML frontmatter that provide
//! reusable instruction sets injected into the agent's system prompt.
//!
//! # Skill file format
//!
//! ```markdown
//! ---
//! name = "code-review"
//! description = "Thorough code review with security and style checks"
//! enabled = true
//! ---
//! When asked to review code, follow this process:
//! 1. Check for security vulnerabilities ...
//! ```
//!
//! # Discovery
//!
//! Skills are loaded from two locations (project overrides global for same name):
//! - `~/.config/rustcode/skills/*.md` — user-global scope
//! - `{workspace}/.rustcode/skills/*.md` — project scope

pub mod loader;
pub mod manager;
pub mod model;

pub use loader::{global_skills_dir, parse_skill_file, project_skills_dir, scan_dir};
pub use manager::SkillsManager;
pub use model::{SkillError, SkillFile, SkillMetadata};

/// Build the `## Available Skills` section for injection into the system prompt.
///
/// Returns an empty string if no enabled skills are present.
#[must_use]
pub fn build_skills_section(skills: &[SkillFile]) -> String {
    let enabled: Vec<&SkillFile> = skills.iter().filter(|s| s.metadata.enabled).collect();
    if enabled.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Available Skills\n\n");
    out.push_str("The following skills are available. Each skill provides specialized instructions for a specific task. Use `/skill <name>` in the TUI or refer to a skill by name in your prompt.\n\n");

    for skill in &enabled {
        out.push_str(&format!(
            "### {} — {}\n\n",
            skill.name(),
            skill.description()
        ));
        if !skill.content.is_empty() {
            out.push_str(&skill.content);
            out.push_str("\n\n");
        }
    }

    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{SkillFile, SkillMetadata};
    use std::path::PathBuf;

    fn make_skill(name: &str, description: &str, content: &str, enabled: bool) -> SkillFile {
        SkillFile {
            metadata: SkillMetadata {
                name: name.to_string(),
                description: description.to_string(),
                enabled,
            },
            content: content.to_string(),
            path: PathBuf::from(format!("/tmp/{name}.md")),
        }
    }

    #[test]
    fn build_section_empty_when_no_skills() {
        assert!(build_skills_section(&[]).is_empty());
    }

    #[test]
    fn build_section_skips_disabled() {
        let skills = vec![make_skill("off", "desc", "content", false)];
        assert!(build_skills_section(&skills).is_empty());
    }

    #[test]
    fn build_section_includes_enabled() {
        let skills = vec![make_skill("review", "Code review", "Step 1", true)];
        let section = build_skills_section(&skills);
        assert!(section.contains("review"));
        assert!(section.contains("Code review"));
        assert!(section.contains("Step 1"));
    }
}
