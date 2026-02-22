use std::path::{Path, PathBuf};

use crate::helpers::{read_json_file, write_json_atomic};
use crate::{push_prompt_history_entry, PromptHistoryFile, PromptHistoryStore, StateError};

impl PromptHistoryStore {
    #[must_use]
    pub fn open_default() -> Self {
        Self {
            path: crate::defaults::default_prompt_history_path(),
        }
    }

    #[must_use]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<String>, StateError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file: PromptHistoryFile = read_json_file(&self.path)?;
        let mut history = Vec::with_capacity(file.prompts.len());
        for prompt in file.prompts {
            let _ = push_prompt_history_entry(&mut history, &prompt);
        }
        Ok(history)
    }

    pub fn save(&self, history: &[String]) -> Result<(), StateError> {
        let mut normalized = Vec::with_capacity(history.len());
        for prompt in history {
            let _ = push_prompt_history_entry(&mut normalized, prompt);
        }
        let file = PromptHistoryFile {
            prompts: normalized,
        };
        write_json_atomic(&self.path, &file)
    }

    pub fn append(&self, prompt: &str) -> Result<bool, StateError> {
        let mut history = self.load()?;
        let changed = push_prompt_history_entry(&mut history, prompt);
        if changed {
            self.save(&history)?;
        }
        Ok(changed)
    }
}
