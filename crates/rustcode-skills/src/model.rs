use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

/// Metadata parsed from a skill file's TOML frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMetadata {
    /// Unique skill name (used with `/skill <name>`).
    pub name: String,
    /// One-line description shown in listings.
    #[serde(default)]
    pub description: String,
    /// Whether this skill is active. Defaults to `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// A loaded skill file: parsed metadata + markdown body content.
#[derive(Debug, Clone)]
pub struct SkillFile {
    pub metadata: SkillMetadata,
    /// Raw markdown body (everything after the closing `---`).
    pub content: String,
    /// Absolute path to the source file.
    pub path: PathBuf,
}

impl SkillFile {
    /// Convenience accessor for the skill name.
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Convenience accessor for the skill description.
    pub fn description(&self) -> &str {
        &self.metadata.description
    }
}

/// Errors that can occur while loading or parsing skill files.
#[derive(Debug, Error)]
pub enum SkillError {
    #[error("missing frontmatter in {path}: file must start with ---")]
    MissingFrontmatter { path: PathBuf },

    #[error("unclosed frontmatter in {path}: no closing ---")]
    UnclosedFrontmatter { path: PathBuf },

    #[error("invalid frontmatter in {path}: {source}")]
    InvalidToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("skill name is empty in {path}")]
    EmptyName { path: PathBuf },
}
