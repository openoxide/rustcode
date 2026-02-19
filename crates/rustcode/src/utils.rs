use crate::cli::Cli;
use anyhow::{Context, Result};
use rustcode_config::{ConfigLoader, ConfigSources};
use rustcode_core::config::ResolvedConfig;
use std::io::{IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn write_stdout_line(line: &str) -> Result<bool> {
    let mut stdout = std::io::stdout();
    if let Err(err) = writeln!(stdout, "{line}") {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(false);
        }
        return Err(anyhow::Error::from(err).context("failed to write to stdout"));
    }
    Ok(true)
}

pub fn write_stdout_raw(text: &str) -> Result<bool> {
    let mut stdout = std::io::stdout();
    if let Err(err) = write!(stdout, "{text}") {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return Ok(false);
        }
        return Err(anyhow::Error::from(err).context("failed to write to stdout"));
    }
    let _ = stdout.flush();
    Ok(true)
}

pub fn is_interactive_terminal() -> bool {
    std::io::stdout().is_terminal()
}

pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn init_tracing() {
    rustcode_logging::init();
}

pub fn load_effective_config(cli: &Cli) -> Result<ResolvedConfig> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut sources = ConfigSources::new(cwd);
    sources.profile_override = cli.profile.clone();
    sources.model_override = cli.model.clone();
    sources.llm_provider_override = cli.llm_provider.clone();
    sources.llm_base_url_override = cli.llm_base_url.clone();
    sources.llm_api_key_env_override = cli.llm_api_key_env.clone();
    sources.trust_project = cli.trust_project_config;

    if cli.allow_network {
        sources.allow_network_override = Some(true);
    } else if cli.deny_network {
        sources.allow_network_override = Some(false);
    }

    ConfigLoader::load(&sources).context("failed to load config")
}

#[cfg(unix)]
pub async fn wait_for_shutdown_signal() -> Result<()> {
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
pub async fn wait_for_shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .map_err(|err| anyhow::anyhow!("failed to listen for Ctrl+C: {err}"))
}
