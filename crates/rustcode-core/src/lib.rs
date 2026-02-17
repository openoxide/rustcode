pub mod command;
pub mod config;
pub mod context;
pub mod error;
pub mod event;
pub mod ports;

pub use command::Command;
pub use config::ResolvedConfig;
pub use context::{CommandContext, SessionMeta};
pub use error::{ConfigError, ExecutionError, PublishError, ShutdownError};
pub use event::{Event, EventId, EventPayload, EventScope};
pub use ports::{CommandExecutor, EventPublisher, PathOperation, PermissionPolicy};
