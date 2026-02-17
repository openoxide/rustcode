use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use rustcode_core::context::CommandContext;
use rustcode_core::event::Event;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin already registered: {0}")]
    Duplicate(String),
    #[error("plugin hook failed: {0}")]
    Hook(String),
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;

    async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
        Ok(())
    }
}

#[derive(Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, Arc<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) -> Result<(), PluginError> {
        let key = plugin.name().to_string();
        if self.plugins.contains_key(&key) {
            return Err(PluginError::Duplicate(key));
        }
        self.plugins.insert(key, plugin);
        Ok(())
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Arc<dyn Plugin>> {
        self.plugins.values()
    }
}
