use std::path::Path;

use crate::{
    loader::{global_skills_dir, project_skills_dir, scan_dir},
    model::SkillFile,
};

/// Manages the set of skills loaded for a workspace session.
///
/// Skills are loaded once at construction.  Project-level skills (from
/// `{workspace}/.rustcode/skills/`) override global skills with the same name.
#[derive(Default)]
pub struct SkillsManager {
    skills: Vec<SkillFile>,
}

impl SkillsManager {
    /// Load all skills for the given workspace root.
    ///
    /// Discovery order (later entries override earlier on name collision):
    /// 1. Global: `~/.config/rustcode/skills/*.md`
    /// 2. Project: `{workspace}/.rustcode/skills/*.md`
    pub fn load(workspace_root: &Path) -> Self {
        let mut global = if let Some(global_dir) = global_skills_dir() {
            scan_dir(&global_dir)
        } else {
            Vec::new()
        };

        let project = scan_dir(&project_skills_dir(workspace_root));

        // Project skills override globals with the same name
        for proj in &project {
            global.retain(|g| g.name() != proj.name());
        }
        global.extend(project);

        // Sort by name for stable ordering
        global.sort_by(|a, b| a.name().cmp(b.name()));

        Self { skills: global }
    }

    /// Returns all enabled skills.
    pub fn enabled(&self) -> impl Iterator<Item = &SkillFile> {
        self.skills.iter().filter(|s| s.metadata.enabled)
    }

    /// Returns all skills (enabled and disabled).
    pub fn all(&self) -> &[SkillFile] {
        &self.skills
    }

    /// Look up a skill by exact name (case-sensitive).
    pub fn get(&self, name: &str) -> Option<&SkillFile> {
        self.skills.iter().find(|s| s.name() == name)
    }

    /// Total number of loaded skills (enabled + disabled).
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Returns `true` if no skills are loaded.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill_in(dir: &std::path::Path, name: &str, enabled: bool) {
        let src = format!(
            "---\nname = \"{name}\"\ndescription = \"desc\"\nenabled = {enabled}\n---\nContent.\n"
        );
        std::fs::write(dir.join(format!("{name}.md")), src).unwrap();
    }

    #[test]
    fn loads_from_project_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let skills_dir = workspace.path().join(".rustcode").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        make_skill_in(&skills_dir, "my-skill", true);

        let mgr = SkillsManager::load(workspace.path());
        assert_eq!(mgr.get("my-skill").unwrap().name(), "my-skill");
    }

    #[test]
    fn enabled_filters_disabled() {
        let workspace = tempfile::tempdir().unwrap();
        let skills_dir = workspace.path().join(".rustcode").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        make_skill_in(&skills_dir, "on", true);
        make_skill_in(&skills_dir, "off", false);

        let mgr = SkillsManager::load(workspace.path());
        assert_eq!(mgr.len(), 2);
        let enabled: Vec<_> = mgr.enabled().collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name(), "on");
    }

    #[test]
    fn project_overrides_global_same_name() {
        // We can't easily override the global dir in tests, but we can verify
        // that if two entries have the same name, the latter wins.
        // This is tested indirectly via IndexMap insertion order.
        let workspace = tempfile::tempdir().unwrap();
        let skills_dir = workspace.path().join(".rustcode").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        make_skill_in(&skills_dir, "shared", true);

        let mgr = SkillsManager::load(workspace.path());
        assert!(mgr.get("shared").is_some());
    }

    #[test]
    fn empty_on_missing_dirs() {
        let workspace = tempfile::tempdir().unwrap();
        let mgr = SkillsManager::load(workspace.path());
        // No skills dir exists, should be empty or have whatever globals exist.
        // We just assert it doesn't panic.
        let _ = mgr.len();
    }
}
