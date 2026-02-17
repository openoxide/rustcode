use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use clap::Parser;
use rustcode_auth::{methods_for_provider, oauth_login_hint, AuthStore, StoredCredential};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use rustcode_config::{ConfigLoader, ConfigSources};
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::ExecutionError;
use rustcode_core::ports::CommandExecutor;
use rustcode_engine::{ChannelPublisher, Engine, WorkspacePermissionPolicy};
use rustcode_io::LocalIo;
use rustcode_llm::build_client;
use rustcode_plugins::PluginRegistry;
use rustcode_tui::TuiApp;

mod cli;
mod render;

use cli::{map_command, AuthCommand, Cli, TopCommand};
use render::{render_event, OutputFormat};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;

    let cli = Cli::parse();
    if let TopCommand::Auth { command } = &cli.command {
        return handle_auth_command(command.clone());
    }
    let launch_tui = matches!(&cli.command, TopCommand::Tui);
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let output_format = OutputFormat::from_json_flag(cli.json);

    let mut config_sources = ConfigSources::new(cwd);
    config_sources.profile_override = cli.profile.clone();
    config_sources.model_override = cli.model.clone();
    config_sources.llm_provider_override = cli.llm_provider.clone();
    config_sources.llm_base_url_override = cli.llm_base_url.clone();
    config_sources.llm_api_key_env_override = cli.llm_api_key_env.clone();
    config_sources.trust_project = cli.trust_project_config;

    let config = ConfigLoader::load(&config_sources).context("failed to load configuration")?;
    let llm_client = build_client(&config).context("failed to initialize llm provider")?;

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

    let io = Arc::new(LocalIo);
    let engine = Engine::new(
        llm_client,
        io.clone(),
        io,
        Arc::new(WorkspacePermissionPolicy),
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

    if launch_tui {
        let _summary = TuiApp::from_domain_receiver(event_rx)
            .run()
            .await
            .context("tui event loop failed")?;
    } else {
        while let Some(event) = event_rx.recv().await {
            println!("{}", render_event(&event, output_format)?);
        }
    }

    match execution_result {
        Ok(()) | Err(ExecutionError::Cancelled) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("command execution failed: {err}")),
    }
}

fn handle_auth_command(command: AuthCommand) -> Result<()> {
    let store = AuthStore::open_default();
    match command {
        AuthCommand::Methods { provider } => {
            let methods = methods_for_provider(&provider);
            let rendered = methods
                .into_iter()
                .map(|method| method.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("provider={provider}");
            println!("methods={rendered}");
        }
        AuthCommand::Status { provider } => {
            let methods = methods_for_provider(&provider);
            let rendered_methods = methods
                .into_iter()
                .map(|method| method.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let credential = match store.get(&provider)? {
                Some(StoredCredential::ApiKey { .. }) => "stored:api_key",
                Some(StoredCredential::OAuth { .. }) => "stored:oauth",
                None => "none",
            };
            println!("provider={provider}");
            println!("methods={rendered_methods}");
            println!("credential={credential}");
            println!("auth_file={}", store.path().display());
        }
        AuthCommand::SetKey { provider, from_env } => {
            let key = std::env::var(&from_env).with_context(|| {
                format!("environment variable {from_env} is not set; cannot store key")
            })?;
            store.set_api_key(&provider, &key)?;
            println!("stored api key for provider={provider}");
            println!("auth_file={}", store.path().display());
        }
        AuthCommand::Remove { provider } => {
            let removed = store.remove(&provider)?;
            println!(
                "{} credential for provider={provider}",
                if removed { "removed" } else { "no stored" }
            );
            println!("auth_file={}", store.path().display());
        }
        AuthCommand::Login { provider } => {
            let methods = methods_for_provider(&provider);
            if !methods
                .iter()
                .any(|method| method.as_str() == "oauth_device_code")
            {
                anyhow::bail!(
                    "provider={provider} does not advertise oauth device login; use `rustcode auth set-key {provider} --from-env <ENV_VAR>`"
                );
            }
            let hint = oauth_login_hint(&provider).ok_or_else(|| {
                anyhow::anyhow!(
                    "oauth login adapter for provider={provider} is not implemented yet"
                )
            })?;
            println!("provider={provider}");
            println!("authorize_url={}", hint.authorize_url);
            println!("instructions={}", hint.instructions);
        }
    }
    Ok(())
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
