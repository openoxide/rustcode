use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::sync::Mutex;

use rustcode_core::config::ResolvedConfig;
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_plugins::{Plugin, PluginError, PluginRegistry};

struct FixturePlugin {
    seen: Arc<Mutex<usize>>,
}

#[async_trait]
impl Plugin for FixturePlugin {
    fn name(&self) -> &'static str {
        "fixture"
    }

    async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
        let mut seen = self.seen.lock().await;
        *seen += 1;
        Ok(())
    }
}

#[tokio::test]
async fn fixture_plugin_receives_dispatched_event() {
    let seen = Arc::new(Mutex::new(0usize));
    let mut registry = PluginRegistry::default();
    registry
        .register(Arc::new(FixturePlugin { seen: seen.clone() }))
        .expect("fixture plugin should register");

    let context = CommandContext::new(
        Arc::new(ResolvedConfig::default()),
        SessionMeta {
            session_id: "fixture-session".to_string(),
            request_id: "fixture-request".to_string(),
            started_at: SystemTime::now(),
        },
    );

    let event = Event::new(1, EventScope::System, EventPayload::Completed);

    for plugin in registry.plugins() {
        plugin
            .on_event(&event, &context)
            .await
            .expect("hook should succeed");
    }

    assert_eq!(*seen.lock().await, 1);
}
