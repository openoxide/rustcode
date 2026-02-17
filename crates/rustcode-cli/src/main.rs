use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use clap::Parser;
use rustcode_auth::{
    known_oauth_providers, methods_for_provider, oauth_login_hint,
    poll_device_code_flow_for_api_key, start_device_code_flow, AuthStore, StoredCredential,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use rustcode_config::{ConfigLoader, ConfigSources};
use rustcode_core::config::ResolvedConfig;
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::ExecutionError;
use rustcode_core::ports::CommandExecutor;
use rustcode_engine::{ChannelPublisher, Engine, WorkspacePermissionPolicy};
use rustcode_io::LocalIo;
use rustcode_llm::{build_client, diagnose_provider, ApiKeySource, ProviderProtocolName};
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
        return handle_auth_command(command.clone()).await;
    }
    if let TopCommand::Models { provider } = &cli.command {
        let config = load_effective_config(&cli)?;
        return handle_models_command(provider.as_deref(), &config, cli.json);
    }
    let launch_tui = matches!(&cli.command, TopCommand::Tui);
    let output_format = OutputFormat::from_json_flag(cli.json);

    let config = load_effective_config(&cli)?;
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
            let rendered = render_event(&event, output_format)?;
            if !write_stdout_line(&rendered)? {
                return Ok(());
            }
        }
    }

    match execution_result {
        Ok(()) | Err(ExecutionError::Cancelled) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("command execution failed: {err}")),
    }
}

async fn handle_auth_command(command: AuthCommand) -> Result<()> {
    let store = AuthStore::open_default();
    match command {
        AuthCommand::List => {
            let providers = store.providers()?;
            if !write_stdout_line(&format!("providers={}", providers.len()))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }

            for provider in providers {
                let credential = match store.get(&provider)? {
                    Some(StoredCredential::ApiKey { .. }) => "stored:api_key",
                    Some(StoredCredential::OAuth { .. }) => "stored:oauth",
                    None => "none",
                };
                if !write_stdout_line(&format!("provider={provider}\tcredential={credential}"))? {
                    return Ok(());
                }
            }
        }
        AuthCommand::Methods { provider } => {
            let methods = methods_for_provider(&provider);
            let rendered = methods
                .into_iter()
                .map(|method| method.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if !write_stdout_line(&format!("provider={provider}"))?
                || !write_stdout_line(&format!("methods={rendered}"))?
            {
                return Ok(());
            }
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
            if !write_stdout_line(&format!("provider={provider}"))?
                || !write_stdout_line(&format!("methods={rendered_methods}"))?
                || !write_stdout_line(&format!("credential={credential}"))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::SetKey { provider, from_env } => {
            let key = std::env::var(&from_env).with_context(|| {
                format!("environment variable {from_env} is not set; cannot store key")
            })?;
            store.set_api_key(&provider, &key)?;
            if !write_stdout_line(&format!("stored api key for provider={provider}"))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::Remove { provider } => {
            let removed = store.remove(&provider)?;
            if !write_stdout_line(&format!(
                "{} credential for provider={provider}",
                if removed { "removed" } else { "no stored" }
            ))? || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::Login {
            provider,
            from_env,
            domain,
            no_wait,
            timeout_secs,
        } => {
            if provider.is_none() {
                if from_env.is_some() {
                    anyhow::bail!("`--from-env` requires a provider: `rustcode auth login <provider> --from-env <ENV_VAR>`");
                }
                return list_auth_login_providers();
            }
            let provider = provider.expect("provider is checked").to_ascii_lowercase();

            if let Some(env_name) = from_env {
                let key = std::env::var(&env_name).with_context(|| {
                    format!("environment variable {env_name} is not set; cannot store key")
                })?;
                store.set_api_key(&provider, &key)?;
                if !write_stdout_line(&format!("stored api key for provider={provider}"))?
                    || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                {
                    return Ok(());
                }
                return Ok(());
            }

            let methods = methods_for_provider(&provider);
            if !methods
                .iter()
                .any(|method| method.as_str() == "oauth_device_code")
            {
                anyhow::bail!(
                    "provider={provider} does not advertise oauth device login; use `rustcode auth login {provider} --from-env <ENV_VAR>`"
                );
            }
            if provider == "github-copilot" || provider == "github-copilot-enterprise" {
                let flow = start_device_code_flow(&provider, domain.as_deref()).await?;
                if !write_stdout_line(&format!("provider={provider}"))?
                    || !write_stdout_line(&format!("authorize_url={}", flow.verification_uri))?
                    || !write_stdout_line(&format!("user_code={}", flow.user_code))?
                    || !write_stdout_line(&format!("interval_secs={}", flow.interval_secs))?
                    || !write_stdout_line(&format!("expires_in_secs={}", flow.expires_in_secs))?
                {
                    return Ok(());
                }

                if no_wait {
                    if !write_stdout_line("status=awaiting_user_authorization")? {
                        return Ok(());
                    }
                    return Ok(());
                }

                if !write_stdout_line("status=polling_for_token")? {
                    return Ok(());
                }
                let timeout = Duration::from_secs(timeout_secs.max(1));
                let token = poll_device_code_flow_for_api_key(&flow, timeout).await?;
                store.set_api_key(&provider, &token)?;
                if !write_stdout_line("status=authorized")?
                    || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                {
                    return Ok(());
                }
            } else {
                let hint = oauth_login_hint(&provider).ok_or_else(|| {
                    anyhow::anyhow!(
                        "oauth login adapter for provider={provider} is not implemented yet"
                    )
                })?;
                if !write_stdout_line(&format!("provider={provider}"))?
                    || !write_stdout_line(&format!("authorize_url={}", hint.authorize_url))?
                    || !write_stdout_line(&format!("instructions={}", hint.instructions))?
                {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ModelsProvider {
    name: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, Value>,
}

fn handle_models_command(
    provider_filter: Option<&str>,
    config: &ResolvedConfig,
    json_output: bool,
) -> Result<()> {
    let index = load_models_index()?;

    if let Some(provider) = provider_filter {
        let Some(entry) = index.get(provider) else {
            anyhow::bail!("provider not found in models index: {provider}");
        };
        let diagnostics = diagnose_provider(config, Some(provider))
            .with_context(|| format!("failed to diagnose provider {provider}"))?;

        let mut model_ids: Vec<String> = entry
            .models
            .keys()
            .map(|id| format!("{provider}/{id}"))
            .collect();
        model_ids.sort();

        if json_output {
            let payload = serde_json::json!({
                "schema_version": 1,
                "provider": {
                    "id": provider,
                    "name": entry.name.clone().unwrap_or_else(|| provider.to_string()),
                    "protocol": render_protocol(&diagnostics.protocol),
                    "base_url": diagnostics.base_url,
                    "endpoint": diagnostics.endpoint,
                    "requires_api_key": diagnostics.requires_api_key,
                    "api_key_source": render_api_key_source(&diagnostics.api_key_source),
                    "api_key_env_candidates": diagnostics.api_key_env_candidates,
                    "missing": diagnostics.missing,
                    "models": model_ids,
                }
            });
            if !write_stdout_line(
                &serde_json::to_string(&payload).context("failed to serialize models json")?,
            )? {
                return Ok(());
            }
            return Ok(());
        }

        if !write_stdout_line(&format!("provider={provider}"))?
            || !write_stdout_line(&format!(
                "protocol={}",
                render_protocol(&diagnostics.protocol)
            ))?
            || !write_stdout_line(&format!(
                "base_url={}",
                diagnostics.base_url.as_deref().unwrap_or("<unset>")
            ))?
            || !write_stdout_line(&format!(
                "endpoint={}",
                diagnostics.endpoint.as_deref().unwrap_or("<unset>")
            ))?
            || !write_stdout_line(&format!(
                "requires_api_key={}",
                diagnostics.requires_api_key
            ))?
            || !write_stdout_line(&format!(
                "api_key_source={}",
                render_api_key_source(&diagnostics.api_key_source)
            ))?
            || !write_stdout_line(&format!(
                "api_key_env_candidates={}",
                if diagnostics.api_key_env_candidates.is_empty() {
                    "<none>".to_string()
                } else {
                    diagnostics.api_key_env_candidates.join(",")
                }
            ))?
            || !write_stdout_line(&format!(
                "missing={}",
                if diagnostics.missing.is_empty() {
                    "<none>".to_string()
                } else {
                    diagnostics.missing.join(",")
                }
            ))?
        {
            return Ok(());
        }

        if !write_stdout_line(&format!("models={}", model_ids.len()))? {
            return Ok(());
        }
        for model_id in model_ids {
            if !write_stdout_line(&model_id)? {
                return Ok(());
            }
        }
        return Ok(());
    }

    let mut providers: Vec<_> = index.into_iter().collect();
    providers.sort_by(|a, b| a.0.cmp(&b.0));

    if json_output {
        let mut rows = Vec::with_capacity(providers.len());
        for (provider_id, entry) in providers {
            let display_name = entry.name.unwrap_or_else(|| provider_id.clone());
            let diagnostics = diagnose_provider(config, Some(&provider_id))
                .with_context(|| format!("failed to diagnose provider {provider_id}"))?;
            rows.push(serde_json::json!({
                "id": provider_id,
                "name": display_name,
                "models": entry.models.len(),
                "protocol": render_protocol(&diagnostics.protocol),
                "endpoint": diagnostics.endpoint,
                "api_key_source": render_api_key_source(&diagnostics.api_key_source),
                "missing": diagnostics.missing,
            }));
        }
        let payload = serde_json::json!({
            "schema_version": 1,
            "providers": rows,
        });
        if !write_stdout_line(
            &serde_json::to_string(&payload).context("failed to serialize models json")?,
        )? {
            return Ok(());
        }
        return Ok(());
    }

    for (provider_id, entry) in providers {
        let display_name = entry.name.unwrap_or_else(|| provider_id.clone());
        let diagnostics = diagnose_provider(config, Some(&provider_id))
            .with_context(|| format!("failed to diagnose provider {provider_id}"))?;
        let missing = if diagnostics.missing.is_empty() {
            "none".to_string()
        } else {
            diagnostics.missing.join("|")
        };
        if !write_stdout_line(&format!(
            "{provider_id}\tmodels={}\tname={display_name}\tprotocol={}\tendpoint={}\tapi_key_source={}\tmissing={missing}",
            entry.models.len(),
            render_protocol(&diagnostics.protocol),
            diagnostics.endpoint.as_deref().unwrap_or("<unset>"),
            render_api_key_source(&diagnostics.api_key_source),
        ))? {
            return Ok(());
        }
    }
    Ok(())
}

fn write_stdout_line(line: &str) -> Result<bool> {
    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{line}") {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(anyhow::anyhow!("stdout write failed: {err}")),
    }
}

fn list_auth_login_providers() -> Result<()> {
    let index = load_models_index();
    let mut rows: Vec<(String, String, String)> = match index {
        Ok(index) => {
            let mut rows = index
                .into_iter()
                .map(|(provider_id, provider)| {
                    let methods = render_auth_methods(&methods_for_provider(&provider_id));
                    let display_name = provider.name.unwrap_or_else(|| provider_id.clone());
                    (provider_id, display_name, methods)
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                auth_login_priority(&a.0)
                    .cmp(&auth_login_priority(&b.0))
                    .then_with(|| a.1.cmp(&b.1))
            });
            rows
        }
        Err(err) => {
            if !write_stdout_line(&format!("warning={err}"))? {
                return Ok(());
            }
            let mut rows = known_oauth_providers()
                .iter()
                .map(|provider| {
                    (
                        (*provider).to_string(),
                        (*provider).to_string(),
                        render_auth_methods(&methods_for_provider(provider)),
                    )
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            rows
        }
    };

    if !write_stdout_line("usage=rustcode auth login <provider> --from-env <ENV_VAR>")?
        || !write_stdout_line(&format!("providers={}", rows.len()))?
    {
        return Ok(());
    }
    for (provider_id, display_name, methods) in rows.drain(..) {
        if !write_stdout_line(&format!(
            "provider={provider_id}\tname={display_name}\tmethods={methods}"
        ))? {
            return Ok(());
        }
    }
    Ok(())
}

fn render_auth_methods(methods: &[rustcode_auth::AuthMethod]) -> String {
    methods
        .iter()
        .map(|method| method.as_str())
        .collect::<Vec<_>>()
        .join("|")
}

fn auth_login_priority(provider_id: &str) -> usize {
    match provider_id {
        "opencode" => 0,
        "anthropic" => 1,
        "github-copilot" => 2,
        "openai" => 3,
        "google" => 4,
        "openrouter" => 5,
        "vercel" => 6,
        _ => 99,
    }
}

fn load_models_index() -> Result<BTreeMap<String, ModelsProvider>> {
    let models_path = resolve_models_path()
        .ok_or_else(|| anyhow::anyhow!("models index not found; set RUSTCODE_MODELS_PATH"))?;
    let raw = std::fs::read_to_string(&models_path)
        .with_context(|| format!("failed to read models index {}", models_path.display()))?;
    serde_json::from_str::<BTreeMap<String, ModelsProvider>>(&raw)
        .with_context(|| format!("failed to parse models index {}", models_path.display()))
}

fn render_protocol(protocol: &ProviderProtocolName) -> &'static str {
    match protocol {
        ProviderProtocolName::Null => "null",
        ProviderProtocolName::OpenAiCompatible => "openai_compatible",
        ProviderProtocolName::AnthropicMessages => "anthropic_messages",
    }
}

fn render_api_key_source(source: &ApiKeySource) -> String {
    match source {
        ApiKeySource::None => "none".to_string(),
        ApiKeySource::ConfigEnvMap { var_name } => format!("config_env:{var_name}"),
        ApiKeySource::ProcessEnv { var_name } => format!("process_env:{var_name}"),
        ApiKeySource::AuthStore => "auth_store".to_string(),
    }
}

fn load_effective_config(cli: &Cli) -> Result<ResolvedConfig> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let mut config_sources = ConfigSources::new(cwd);
    config_sources.profile_override = cli.profile.clone();
    config_sources.model_override = cli.model.clone();
    config_sources.llm_provider_override = cli.llm_provider.clone();
    config_sources.llm_base_url_override = cli.llm_base_url.clone();
    config_sources.llm_api_key_env_override = cli.llm_api_key_env.clone();
    config_sources.trust_project = cli.trust_project_config;
    ConfigLoader::load(&config_sources).context("failed to load configuration")
}

fn resolve_models_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUSTCODE_MODELS_PATH") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        let candidate = PathBuf::from(path).join("opencode/models.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(path) = std::env::var("HOME") {
        let candidate = PathBuf::from(path).join(".cache/opencode/models.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
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
