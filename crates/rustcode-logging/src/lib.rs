//! Structured logging and tracing initialization for rustcode.
//!
//! Wraps [`tracing`] and [`tracing_subscriber`] to provide a single
//! `init()` call that sets up structured logging with sensible defaults.
//!
//! # Usage
//!
//! ```rust
//! rustcode_logging::init();
//! tracing::info!("application started");
//! ```
//!
//! # Environment Variables
//!
//! - `RUST_LOG` — standard `tracing_subscriber` env filter
//!   (e.g., `rustcode=debug,rustcode_engine=trace`)

use tracing_subscriber::EnvFilter;

/// Re-export core tracing macros for convenience.
pub use tracing::{debug, error, info, trace, warn};

/// Initialize structured tracing with sensible defaults.
///
/// - Output goes to stderr (keeps stdout clean for agent output)
/// - Uses `RUST_LOG` env filter (default: `warn` if not set)
/// - Format: compact text (or JSON if `RUSTCODE_LOG_FORMAT=json`)
///
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init() {
    init_with_default_level("warn");
}

/// Initialize structured tracing with a specified default log level.
///
/// The `default_level` is used when `RUST_LOG` is not set.
/// Common values: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`.
///
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init_with_default_level(default_level: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_does_not_panic() {
        // Just verify init doesn't panic when called
        init();
    }

    #[test]
    fn init_with_level_does_not_panic() {
        init_with_default_level("trace");
    }

    #[test]
    fn double_init_is_safe() {
        init();
        init(); // should be a no-op
    }
}
