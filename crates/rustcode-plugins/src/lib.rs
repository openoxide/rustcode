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

    pub fn unregister(&mut self, name: &str) -> bool {
        self.plugins.remove(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[derive(Default)]
    struct AlphaPlugin;

    #[async_trait]
    impl Plugin for AlphaPlugin {
        fn name(&self) -> &'static str {
            "alpha"
        }
    }

    #[test]
    fn register_duplicate_is_rejected() {
        let mut registry = PluginRegistry::default();
        registry
            .register(Arc::new(AlphaPlugin))
            .expect("first registration should pass");

        let duplicate = registry
            .register(Arc::new(AlphaPlugin))
            .expect_err("duplicate registration must fail");
        assert!(matches!(duplicate, PluginError::Duplicate(name) if name == "alpha"));
    }

    #[test]
    fn unregister_updates_registry_size() {
        let mut registry = PluginRegistry::default();
        assert!(registry.is_empty());

        registry
            .register(Arc::new(AlphaPlugin))
            .expect("registration should pass");
        assert_eq!(registry.len(), 1);

        assert!(registry.unregister("alpha"));
        assert!(registry.is_empty());
        assert!(!registry.unregister("alpha"));
    }
}
