//! Structured logging and tracing initialization for rustcode.
//!
//! Wraps [`tracing`] and [`tracing_subscriber`] to provide a single
//! `init()` call that sets up structured logging with sensible defaults,
//! and an optional `init_with_otel()` that also exports spans and logs via
//! OTLP HTTP to a given endpoint.
//!
//! # Usage
//!
//! ```rust
//! // Basic setup (stderr, warn level by default)
//! rustcode_logging::init();
//! tracing::info!("application started");
//!
//! // With OpenTelemetry export
//! let _guard = rustcode_logging::init_with_otel(Some("http://localhost:4318"));
//! ```
//!
//! # Environment Variables
//!
//! - `RUST_LOG` — standard `tracing_subscriber` env filter
//!   (e.g., `rustcode=debug,rustcode_engine=trace`)
//! - `RUSTCODE_OTEL_ENDPOINT` — OTLP HTTP endpoint; enables OpenTelemetry export when set

use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Re-export core tracing macros for convenience.
pub use tracing::{debug, error, info, trace, warn};

// ── OtelGuard ────────────────────────────────────────────────────────────────

/// RAII guard that shuts down OpenTelemetry providers on drop.
///
/// Hold this value until the process is ready to exit so all pending spans
/// and log records are flushed before shutdown.
pub struct OtelGuard {
    tracer_provider: opentelemetry_sdk::trace::SdkTracerProvider,
    logger_provider: opentelemetry_sdk::logs::SdkLoggerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(e) = self.tracer_provider.shutdown() {
            eprintln!("OTel tracer provider shutdown error: {e}");
        }
        if let Err(e) = self.logger_provider.shutdown() {
            eprintln!("OTel logger provider shutdown error: {e}");
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialize structured tracing with sensible defaults.
///
/// - Output goes to stderr (keeps stdout clean for agent output)
/// - Uses `RUST_LOG` env filter (default: `warn` if not set)
/// - Format: compact text
///
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init() {
    let _ = init_with_otel(None);
}

/// Initialize tracing in silent mode — all output suppressed.
///
/// Used when running the interactive TUI so that log messages do not bleed
/// through the ratatui alternate-screen terminal buffer.  The global
/// subscriber slot is claimed (so no later init can override it), but the
/// effective filter is `"off"` — zero messages are ever written.
///
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init_silent() {
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(EnvFilter::new("off")),
        )
        .try_init();
}

/// Initialize structured tracing with a specified default log level.
///
/// The `default_level` is used when `RUST_LOG` is not set.
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init_with_default_level(default_level: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_writer(std::io::stderr)
                .with_target(true)
                .with_thread_ids(false)
                .with_filter(filter),
        )
        .try_init();
}

/// Initialize tracing with optional OpenTelemetry export.
///
/// If `otel_endpoint` is `Some(url)`, OTLP-HTTP trace and log exporters are
/// configured pointing at that URL. Provider construction is wrapped in
/// `catch_unwind` because the OpenTelemetry SDK can panic on bad configuration — in
/// that case OpenTelemetry is silently disabled and only stderr logging is active.
///
/// Returns `Some(OtelGuard)` when OpenTelemetry is successfully initialized; `None`
/// otherwise. Hold the guard until process exit to flush all pending records.
///
/// Safe to call multiple times — the tracing subscriber init is a no-op after
/// the first successful call.
pub fn init_with_otel(otel_endpoint: Option<&str>) -> Option<OtelGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_thread_ids(false)
        .with_filter(filter);

    // Attempt to build OTel providers if endpoint is given.
    // catch_unwind protects against panics inside the OTel SDK (known to
    // occur with certain OTLP configuration or missing tokio runtime context).
    let otel_result = otel_endpoint.and_then(|endpoint| {
        let endpoint = endpoint.to_owned();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            build_otel_providers(&endpoint)
        }))
        .ok()
        .and_then(|r| r.ok())
    });

    if let Some((tracer_provider, logger_provider)) = otel_result {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let tracer = tracer_provider.tracer("rustcode");
        let otel_trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        let otel_log_layer = opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(
            &logger_provider,
        );

        let _ = tracing_subscriber::registry()
            .with(fmt_layer)
            .with(otel_trace_layer)
            .with(otel_log_layer)
            .try_init();

        Some(OtelGuard {
            tracer_provider,
            logger_provider,
        })
    } else {
        let _ = tracing_subscriber::registry().with(fmt_layer).try_init();
        None
    }
}

// ── Internal ─────────────────────────────────────────────────────────────────

type OtelProviders = (
    opentelemetry_sdk::trace::SdkTracerProvider,
    opentelemetry_sdk::logs::SdkLoggerProvider,
);

/// Build OTLP HTTP trace and log providers for the given endpoint.
fn build_otel_providers(endpoint: &str) -> Result<OtelProviders, Box<dyn std::error::Error>> {
    use opentelemetry_otlp::{LogExporter, SpanExporter, WithExportConfig};
    use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider};

    let span_exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{endpoint}/v1/traces"))
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .build();

    let log_exporter = LogExporter::builder()
        .with_http()
        .with_endpoint(format!("{endpoint}/v1/logs"))
        .build()?;

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .build();

    Ok((tracer_provider, logger_provider))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_does_not_panic() {
        init();
    }

    #[test]
    fn init_with_level_does_not_panic() {
        init_with_default_level("trace");
    }

    #[test]
    fn double_init_is_safe() {
        init();
        init(); // second call is a no-op
    }

    #[test]
    fn init_with_otel_none_does_not_panic() {
        let guard = init_with_otel(None);
        assert!(guard.is_none());
    }

    #[test]
    fn init_with_otel_bad_endpoint_does_not_panic() {
        // Bad endpoint — build_otel_providers may fail or succeed lazily.
        // Either way no panic should escape.
        let _ = init_with_otel(Some("http://127.0.0.1:1"));
    }
}
