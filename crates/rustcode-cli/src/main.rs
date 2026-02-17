use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use rustcode_config::{ConfigLoader, ConfigSources};
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::ExecutionError;
use rustcode_core::ports::CommandExecutor;
use rustcode_engine::{ChannelPublisher, Engine};
use rustcode_io::LocalIo;
use rustcode_llm::NullLlmClient;
use rustcode_plugins::PluginRegistry;

mod cli;
mod render;

use cli::{map_command, Cli};
use render::{render_event, OutputFormat};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;

    let cli = Cli::parse();
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let output_format = OutputFormat::from_json_flag(cli.json);

    let mut config_sources = ConfigSources::new(cwd);
    config_sources.profile_override = cli.profile.clone();
    config_sources.model_override = cli.model.clone();
    config_sources.trust_project = cli.trust_project_config;

    let config = ConfigLoader::load(&config_sources).context("failed to load configuration")?;

    let cancellation = CancellationToken::new();
    let context = CommandContext::with_cancellation(
        Arc::new(config),
        SessionMeta {
            session_id: "session-1".to_string(),
            request_id: "request-1".to_string(),
            started_at: SystemTime::now(),
        },
        cancellation.clone(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(512);
    let publisher = Arc::new(ChannelPublisher::new(event_tx));

    let engine = Engine::new(
        Arc::new(NullLlmClient),
        Arc::new(LocalIo),
        PluginRegistry::default(),
    );

    let command = map_command(cli.command);
    let execution = engine.execute(command, context, publisher.clone());
    tokio::pin!(execution);

    let execution_result = tokio::select! {
        result = &mut execution => result,
        result = wait_for_shutdown_signal() => {
            result.context("failed to receive shutdown signal")?;
            cancellation.cancel();
            execution.await
        }
    };

    drop(publisher);

    while let Some(event) = event_rx.recv().await {
        println!("{}", render_event(&event, output_format)?);
    }

    match execution_result {
        Ok(()) | Err(ExecutionError::Cancelled) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("command execution failed: {err}")),
    }
}

fn init_tracing() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init()
        .map_err(|err| anyhow::anyhow!("failed to initialize tracing: {err}"))
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = signal(SignalKind::terminate())
        .map_err(|err| anyhow::anyhow!("failed to listen for SIGTERM: {err}"))?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|err| anyhow::anyhow!("failed to listen for Ctrl+C: {err}"))
}
