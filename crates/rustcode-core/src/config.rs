use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub profile: String,
    pub workspace_root: PathBuf,
    pub allow_network: bool,
    pub model: String,
    pub plugins: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            profile: "default".to_string(),
            workspace_root: PathBuf::from("."),
            allow_network: false,
            model: "default".to_string(),
            plugins: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}
