use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use rustcode_auth::{
    complete_browser_oauth_flow, complete_mcp_browser_oauth_flow, discover_mcp_oauth,
    known_oauth_providers, methods_for_provider, oauth_login_hint,
    poll_device_code_flow_for_credential, start_browser_oauth_flow, start_device_code_flow,
    start_mcp_browser_oauth_flow, AuthMethod, AuthStore, StoredCredential,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use rustcode_config::{
    edit_mcp_server, remove_mcp_server, ConfigEditScope, ConfigLoader, ConfigSources,
};
use rustcode_core::config::{McpServerConfig as CoreMcpServerConfig, ResolvedConfig};
use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::ExecutionError;
use rustcode_core::event::EventPayload;
use rustcode_core::ports::CommandExecutor;
use rustcode_engine::{ChannelPublisher, Engine, WorkspacePermissionPolicy};
use rustcode_io::LocalIo;
use rustcode_llm::{
    build_client, builtin_provider_ids, derive_copilot_enterprise_base_url, diagnose_provider,
    ApiKeySource, ProviderProtocolName,
};
use rustcode_plugins::PluginRegistry;
use rustcode_state::{FileTranscriptRecorder, SessionStore};
use rustcode_tui::TuiApp;

mod cli;
mod render;

use cli::{AuthCommand, Cli, McpCommand, SessionCommand, TopCommand};
use render::{render_event, OutputFormat};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;

    let cli = Cli::parse();

    if matches!(&cli.command, TopCommand::Version) {
        let version = env!("CARGO_PKG_VERSION");
        if cli.json {
            let payload = serde_json::json!({
                "schema_version": 1,
                "command": "version",
                "version": version,
            });
            write_stdout_line(
                &serde_json::to_string(&payload).context("failed to serialize version json")?,
            )?;
        } else {
            write_stdout_line(version)?;
        }
        return Ok(());
    }

    if let TopCommand::Auth { command } = &cli.command {
        return match handle_auth_command(command.clone(), cli.json).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if cli.json {
                    if let AuthCommand::Login {
                        provider, method, ..
                    } = command
                    {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "auth.login",
                            "provider": provider.as_deref(),
                            "method": method.as_deref(),
                            "stage": "failed",
                            "error_kind": classify_auth_error(&err),
                            "error": err.to_string(),
                        });
                        if let Ok(serialized) = serde_json::to_string(&payload) {
                            let _ = write_stdout_line(&serialized);
                        }
                    }
                }
                Err(err)
            }
        };
    }
    if let TopCommand::Mcp { command } = &cli.command {
        let config_result = load_effective_config(&cli);
        return match config_result {
            Ok(config) => match handle_mcp_command(command.clone(), cli.json, &config).await {
                Ok(()) => Ok(()),
                Err(err) => {
                    if cli.json {
                        if let McpCommand::Login { name, url, .. } = command {
                            let payload = serde_json::json!({
                                "schema_version": 1,
                                "command": "mcp.login",
                                "name": name,
                                "url": url,
                                "stage": "failed",
                                "error_kind": classify_mcp_error(&err),
                                "error": err.to_string(),
                            });
                            if let Ok(serialized) = serde_json::to_string(&payload) {
                                let _ = write_stdout_line(&serialized);
                            }
                        }
                    }
                    Err(err)
                }
            },
            Err(err) => {
                if cli.json {
                    if let McpCommand::Login { name, url, .. } = command {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "mcp.login",
                            "name": name,
                            "url": url,
                            "stage": "failed",
                            "error_kind": classify_mcp_error(&err),
                            "error": err.to_string(),
                        });
                        if let Ok(serialized) = serde_json::to_string(&payload) {
                            let _ = write_stdout_line(&serialized);
                        }
                    }
                }
                Err(err)
            }
        };
    }
    if let TopCommand::Models { provider } = &cli.command {
        let config = load_effective_config(&cli)?;
        return handle_models_command(provider.as_deref(), &config, cli.json);
    }
    if let TopCommand::Session { command } = &cli.command {
        let config = load_effective_config(&cli)?;
        return handle_session_command(command.clone(), cli.json, &config);
    }
    let launch_tui = matches!(&cli.command, TopCommand::Tui);
    let requires_llm = matches!(
        &cli.command,
        TopCommand::Run { .. } | TopCommand::Agent { .. } | TopCommand::Serve { .. }
    );
    let output_format = OutputFormat::from_json_flag(cli.json);
    let event_debug = cli.event_debug;

    let config = load_effective_config(&cli)?;

    let session_store = SessionStore::open_default();
    let (session_info, run_history, agent_history) = match &cli.command {
        TopCommand::Run {
            prompt: _,
            continue_session,
            session,
            fork,
            title,
        } => {
            let (info, history) = resolve_session(
                &session_store,
                &config,
                *continue_session,
                session.as_deref(),
                *fork,
                title.clone(),
            )?;
            (Some(info), history, Vec::new())
        }
        TopCommand::Agent {
            prompt: _,
            continue_session,
            session,
            fork,
            title,
            ..
        } => {
            let (info, history) = resolve_session(
                &session_store,
                &config,
                *continue_session,
                session.as_deref(),
                *fork,
                title.clone(),
            )?;
            (Some(info), Vec::new(), history)
        }
        _ => (None, Vec::new(), Vec::new()),
    };

    let session_id = session_info
        .as_ref()
        .map(|info| info.id.clone())
        .unwrap_or_else(|| "session-1".to_string());

    let run_user_prompt = match &cli.command {
        TopCommand::Run { prompt, .. } => Some(prompt.clone()),
        _ => None,
    };
    let run_session_id = if run_user_prompt.is_some() {
        session_info.as_ref().map(|info| info.id.clone())
    } else {
        None
    };

    let request_id = {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("request-{now}")
    };

    let recorder: Option<Arc<dyn rustcode_core::TranscriptRecorder>> =
        if matches!(&cli.command, TopCommand::Agent { .. }) {
            Some(Arc::new(FileTranscriptRecorder::new(session_store.clone())))
        } else {
            None
        };

    let approver: Option<Arc<dyn rustcode_core::ToolApprover>> = if matches!(
        &cli.command,
        TopCommand::Agent {
            allow_write: true,
            ..
        } | TopCommand::Agent {
            allow_edit: true,
            ..
        } | TopCommand::Agent {
            allow_exec: true,
            ..
        }
    ) {
        if is_interactive_terminal() {
            Some(Arc::new(StdioToolApprover))
        } else {
            None
        }
    } else {
        None
    };

    let llm_client = if requires_llm {
        build_client(&config).context("failed to initialize llm provider")?
    } else {
        Arc::new(rustcode_llm::NullLlmClient)
    };

    let cancellation = CancellationToken::new();
    let context = CommandContext::with_cancellation(
        Arc::new(config),
        SessionMeta {
            session_id,
            request_id,
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
        recorder,
        approver,
    );
    let command = match cli.command {
        TopCommand::Run { prompt, .. } => {
            let prompt = if run_history.is_empty() {
                prompt
            } else {
                let mut rendered = String::new();
                for msg in &run_history {
                    let Some(text) = msg.content.as_str() else {
                        continue;
                    };
                    let role = match msg.role {
                        rustcode_core::MessageRole::System => "System",
                        rustcode_core::MessageRole::User => "User",
                        rustcode_core::MessageRole::Assistant => "Assistant",
                        rustcode_core::MessageRole::Tool => "Tool",
                    };
                    rendered.push_str(role);
                    rendered.push_str(": ");
                    rendered.push_str(text);
                    rendered.push('\n');
                }
                rendered.push_str("User: ");
                rendered.push_str(&prompt);
                rendered.push_str("\nAssistant:");
                rendered
            };
            rustcode_core::Command::Run { prompt }
        }
        TopCommand::Agent {
            prompt,
            max_steps,
            max_tool_calls_per_step,
            allow_write,
            allow_edit,
            allow_exec,
            max_read_bytes,
            max_list_entries,
            max_tool_result_bytes,
            max_write_bytes,
            ..
        } => rustcode_core::Command::Agent {
            prompt,
            options: rustcode_core::command::AgentOptions {
                max_steps,
                max_tool_calls_per_step,
                allow_write,
                allow_edit,
                allow_exec,
                max_read_bytes,
                max_list_entries,
                max_tool_result_bytes,
                max_write_bytes,
            },
            history: agent_history,
        },
        TopCommand::Exec { command, args } => rustcode_core::Command::Exec { command, args },
        TopCommand::List { path } => rustcode_core::Command::List { path },
        TopCommand::Read { path } => rustcode_core::Command::Read { path },
        TopCommand::Write { path, contents } => rustcode_core::Command::Write { path, contents },
        TopCommand::Edit { path, from, to } => rustcode_core::Command::Edit { path, from, to },
        TopCommand::Tui => rustcode_core::Command::Tui,
        TopCommand::Serve { listen } => rustcode_core::Command::Serve { listen },
        TopCommand::Version => rustcode_core::Command::Version,
        TopCommand::Models { .. } => {
            anyhow::bail!("internal error: models command must be handled before engine dispatch");
        }
        TopCommand::Auth { .. } => {
            anyhow::bail!("internal error: auth command must be handled before engine dispatch");
        }
        TopCommand::Mcp { .. } => {
            anyhow::bail!("internal error: mcp command must be handled before engine dispatch");
        }
        TopCommand::Session { .. } => {
            anyhow::bail!("internal error: session command must be handled before engine dispatch");
        }
    };
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
        let mut streamed_text_open = false;
        let mut run_output_capture = String::new();
        while let Some(event) = event_rx.recv().await {
            if matches!(output_format, OutputFormat::Human) && !event_debug {
                match &event.payload {
                    EventPayload::OutputChunk { text } => {
                        if run_session_id.is_some()
                            && matches!(event.scope, rustcode_core::EventScope::Command)
                        {
                            run_output_capture.push_str(text);
                        }
                        if !write_stdout_raw(text)? {
                            return Ok(());
                        }
                        streamed_text_open = true;
                        continue;
                    }
                    EventPayload::Completed => {
                        if streamed_text_open {
                            if !write_stdout_raw("\n")? {
                                return Ok(());
                            }
                            streamed_text_open = false;
                        }
                        continue;
                    }
                    _ => {
                        if streamed_text_open {
                            if !write_stdout_raw("\n")? {
                                return Ok(());
                            }
                            streamed_text_open = false;
                        }
                    }
                }
            }
            let rendered = render_event(&event, output_format)?;
            if !write_stdout_line(&rendered)? {
                return Ok(());
            }
        }
        if streamed_text_open {
            let _ = write_stdout_raw("\n")?;
        }

        if let (Some(session_id), Some(user_prompt)) = (run_session_id.as_deref(), run_user_prompt)
        {
            if matches!(execution_result, Ok(())) {
                let store = session_store.clone();
                let user = rustcode_core::StoredMessage {
                    id: store.new_message_id(),
                    role: rustcode_core::MessageRole::User,
                    created_at_unix_ms: now_unix_ms(),
                    content: serde_json::Value::String(user_prompt),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                };
                let assistant_text = run_output_capture.trim_end().to_string();
                let assistant = rustcode_core::StoredMessage {
                    id: store.new_message_id(),
                    role: rustcode_core::MessageRole::Assistant,
                    created_at_unix_ms: now_unix_ms(),
                    content: serde_json::Value::String(assistant_text),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                };

                store
                    .append_message(session_id, &user)
                    .context("failed to store run user message")?;
                store
                    .append_message(session_id, &assistant)
                    .context("failed to store run assistant message")?;
            }
        }
    }

    match execution_result {
        Ok(()) | Err(ExecutionError::Cancelled) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("command execution failed: {err}")),
    }
}

async fn handle_auth_command(command: AuthCommand, json_output: bool) -> Result<()> {
    let store = AuthStore::open_default();
    match command {
        AuthCommand::List => {
            let providers = store.providers()?;
            if json_output {
                let mut rows = Vec::with_capacity(providers.len());
                for provider in providers {
                    let credential = render_stored_credential(store.get(&provider)?);
                    rows.push(serde_json::json!({
                        "id": provider,
                        "credential": credential,
                    }));
                }
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "auth_file": store.path().display().to_string(),
                    "providers": rows,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize auth list json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }
            if !write_stdout_line(&format!("providers={}", providers.len()))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }

            for provider in providers {
                let credential = render_stored_credential(store.get(&provider)?);
                if !write_stdout_line(&format!("provider={provider}\tcredential={credential}"))? {
                    return Ok(());
                }
            }
        }
        AuthCommand::Methods { provider } => {
            if let Some(provider) = provider {
                let methods = methods_for_provider(&provider);
                if json_output {
                    let payload = serde_json::json!({
                        "schema_version": 1,
                        "provider": &provider,
                        "methods": methods
                            .iter()
                            .map(|method| method.as_str())
                            .collect::<Vec<_>>(),
                    });
                    if !write_stdout_line(
                        &serde_json::to_string(&payload)
                            .context("failed to serialize auth methods json")?,
                    )? {
                        return Ok(());
                    }
                    return Ok(());
                }
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
            } else {
                list_auth_methods(json_output)?;
            }
        }
        AuthCommand::Status { provider } => {
            let methods = methods_for_provider(&provider);
            let method_names = methods
                .into_iter()
                .map(|method| method.as_str().to_string())
                .collect::<Vec<_>>();
            let stored = store.get(&provider)?;
            let credential = render_stored_credential(stored.clone());
            let domain = match stored {
                Some(StoredCredential::ApiKey { domain, .. }) => domain,
                Some(StoredCredential::OAuth { domain, .. }) => domain,
                None => None,
            };
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "provider": &provider,
                    "methods": method_names,
                    "credential": credential,
                    "domain": domain,
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize auth status json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            let rendered_methods = method_names.join(", ");
            if !write_stdout_line(&format!("provider={provider}"))?
                || !write_stdout_line(&format!("methods={rendered_methods}"))?
                || !write_stdout_line(&format!("credential={credential}"))?
                || !write_stdout_line(&format!(
                    "domain={}",
                    domain.as_deref().unwrap_or("<unset>")
                ))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::SetKey {
            provider,
            from_env,
            domain,
        } => {
            let key = std::env::var(&from_env).with_context(|| {
                format!("environment variable {from_env} is not set; cannot store key")
            })?;

            let normalized_domain = if let Some(domain) = domain.as_deref() {
                if provider != "github-copilot-enterprise" {
                    anyhow::bail!(
                        "--domain is only supported for provider=github-copilot-enterprise; received provider={provider}"
                    );
                }
                Some(
                    rustcode_auth::normalize_domain(domain)
                        .context("failed to normalize domain")?,
                )
            } else {
                None
            };

            store.set_api_key_with_domain(&provider, &key, normalized_domain.as_deref())?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "provider": &provider,
                    "action": "set_key",
                    "domain": normalized_domain,
                    "credential": "stored:api_key",
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize auth set-key json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }
            if !write_stdout_line(&format!("stored api key for provider={provider}"))?
                || !write_stdout_line(&format!(
                    "domain={}",
                    normalized_domain.as_deref().unwrap_or("<unset>")
                ))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::SetOauth {
            provider,
            access_env,
            refresh_env,
            expires_unix,
            account_id,
        } => {
            let access_token = std::env::var(&access_env).with_context(|| {
                format!(
                    "environment variable {access_env} is not set; cannot store oauth access token"
                )
            })?;
            let refresh_token = refresh_env
                .as_ref()
                .map(|name| {
                    std::env::var(name).with_context(|| {
                        format!(
                            "environment variable {name} is not set; cannot store oauth refresh token"
                        )
                    })
                })
                .transpose()?;
            store.set_oauth(
                &provider,
                &access_token,
                refresh_token.as_deref(),
                expires_unix,
                account_id.as_deref(),
            )?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "provider": &provider,
                    "action": "set_oauth",
                    "credential": "stored:oauth",
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize auth set-oauth json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }
            if !write_stdout_line(&format!("stored oauth credential for provider={provider}"))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
        AuthCommand::Remove { provider } => {
            let removed = store.remove(&provider)?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "provider": &provider,
                    "action": "remove",
                    "removed": removed,
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize auth remove json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }
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
            method,
            domain,
            oauth_port,
            no_wait,
            timeout_secs,
        } => {
            let provider = match provider {
                Some(provider) => provider,
                None => {
                    if from_env.is_some() {
                        anyhow::bail!("`--from-env` requires a provider: `rustcode auth login <provider> --from-env <ENV_VAR>`");
                    }
                    if method.is_some() {
                        anyhow::bail!("`--method` requires a provider: `rustcode auth login <provider> --method <method>`");
                    }

                    if json_output || !is_interactive_terminal() {
                        return list_auth_login_providers(json_output);
                    }

                    prompt_for_login_provider()?
                }
            };
            let provider = provider.to_ascii_lowercase();
            let methods = methods_for_provider(&provider);
            let selected_method = resolve_login_method_with_context(
                &provider,
                method.as_deref(),
                &methods,
                from_env.is_some(),
            )
            .with_context(|| format!("unsupported auth method for provider={provider}"))?;

            if let Some(env_name) = from_env {
                if selected_method != AuthMethod::ApiKey {
                    anyhow::bail!(
                        "`--from-env` can only be used with method=api_key; received method={}",
                        selected_method.as_str()
                    );
                }
                let key = std::env::var(&env_name).with_context(|| {
                    format!("environment variable {env_name} is not set; cannot store key")
                })?;
                let normalized_domain = if provider == "github-copilot-enterprise" {
                    domain
                        .as_deref()
                        .map(rustcode_auth::normalize_domain)
                        .transpose()
                        .context("failed to normalize domain")?
                } else {
                    None
                };
                store.set_api_key_with_domain(&provider, &key, normalized_domain.as_deref())?;
                if json_output {
                    let payload = serde_json::json!({
                        "schema_version": 1,
                        "provider": &provider,
                        "method": selected_method.as_str(),
                        "stage": "authorized",
                        "source": "env",
                        "credential": "stored:api_key",
                        "domain": normalized_domain,
                        "auth_file": store.path().display().to_string(),
                    });
                    if !write_stdout_line(
                        &serde_json::to_string(&payload)
                            .context("failed to serialize auth login json")?,
                    )? {
                        return Ok(());
                    }
                    return Ok(());
                }
                if !write_stdout_line(&format!("stored api key for provider={provider}"))?
                    || !write_stdout_line(&format!(
                        "domain={}",
                        normalized_domain.as_deref().unwrap_or("<unset>")
                    ))?
                    || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                {
                    return Ok(());
                }
                return Ok(());
            }

            match selected_method {
                AuthMethod::ApiKey => {
                    print_provider_api_key_hint(&provider)?;
                    let key = prompt_for_api_key(&provider)?;
                    let normalized_domain = if provider == "github-copilot-enterprise" {
                        domain
                            .as_deref()
                            .map(rustcode_auth::normalize_domain)
                            .transpose()
                            .context("failed to normalize domain")?
                    } else {
                        None
                    };
                    store.set_api_key_with_domain(&provider, &key, normalized_domain.as_deref())?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "authorized",
                            "source": "prompt",
                            "credential": "stored:api_key",
                            "domain": normalized_domain,
                            "auth_file": store.path().display().to_string(),
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                        return Ok(());
                    }
                    if !write_stdout_line(&format!("stored api key for provider={provider}"))?
                        || !write_stdout_line(&format!(
                            "domain={}",
                            normalized_domain.as_deref().unwrap_or("<unset>")
                        ))?
                        || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                    {
                        return Ok(());
                    }
                }
                AuthMethod::OAuthDeviceCode => {
                    let flow = start_device_code_flow(&provider, domain.as_deref()).await?;
                    let copilot_enterprise_llm_base_url_hint = if provider
                        == "github-copilot-enterprise"
                        && domain
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                    {
                        Some(derive_copilot_enterprise_base_url(&flow.domain))
                    } else {
                        None
                    };
                    let copilot_enterprise_env_hint =
                        copilot_enterprise_llm_base_url_hint.as_ref().map(|_| {
                            format!("export GITHUB_COPILOT_ENTERPRISE_DOMAIN={}", flow.domain)
                        });
                    let copilot_enterprise_run_hint = copilot_enterprise_llm_base_url_hint.as_ref().map(|base_url| {
                        format!(
                            "RUSTCODE_ALLOW_NETWORK=1 rustcode --llm-provider github-copilot-enterprise --llm-base-url \"{base_url}\" --model github-copilot/gpt-4o run \"hello\""
                        )
                    });
                    if json_output {
                        let challenge_payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "challenge",
                            "oauth_domain": flow.domain,
                            "authorize_url": flow.verification_uri,
                            "user_code": flow.user_code,
                            "interval_secs": flow.interval_secs,
                            "expires_in_secs": flow.expires_in_secs,
                            "llm_base_url_hint": copilot_enterprise_llm_base_url_hint,
                            "env_hint": copilot_enterprise_env_hint,
                            "run_hint": copilot_enterprise_run_hint,
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&challenge_payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line(&format!("provider={provider}"))?
                        || !write_stdout_line(&format!("method={}", selected_method.as_str()))?
                        || !write_stdout_line(&format!("authorize_url={}", flow.verification_uri))?
                        || !write_stdout_line(&format!("user_code={}", flow.user_code))?
                        || !write_stdout_line(&format!("oauth_domain={}", flow.domain))?
                        || !write_stdout_line(&format!("interval_secs={}", flow.interval_secs))?
                        || !write_stdout_line(&format!("expires_in_secs={}", flow.expires_in_secs))?
                    {
                        return Ok(());
                    }

                    if let Some(base_url) = copilot_enterprise_llm_base_url_hint.as_deref() {
                        if !write_stdout_line(&format!("llm_base_url_hint={base_url}"))? {
                            return Ok(());
                        }
                    }
                    if let Some(hint) = copilot_enterprise_env_hint.as_deref() {
                        if !write_stdout_line(&format!("env_hint={hint}"))? {
                            return Ok(());
                        }
                    }
                    if let Some(hint) = copilot_enterprise_run_hint.as_deref() {
                        if !write_stdout_line(&format!("run_hint={hint}"))? {
                            return Ok(());
                        }
                    }

                    if no_wait {
                        if json_output {
                            let payload = serde_json::json!({
                                "schema_version": 1,
                                "provider": &provider,
                                "method": selected_method.as_str(),
                                "stage": "awaiting_user_authorization",
                            });
                            if !write_stdout_line(
                                &serde_json::to_string(&payload)
                                    .context("failed to serialize auth login json")?,
                            )? {
                                return Ok(());
                            }
                        } else if !write_stdout_line("status=awaiting_user_authorization")? {
                            return Ok(());
                        }
                        return Ok(());
                    }

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "polling_for_token",
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("status=polling_for_token")? {
                        return Ok(());
                    }
                    let timeout = Duration::from_secs(timeout_secs.max(1));
                    let credential = poll_device_code_flow_for_credential(&flow, timeout).await?;
                    let credential_kind = login_credential_kind(&credential);
                    let copilot_enterprise_domain_for_store = if provider
                        == "github-copilot-enterprise"
                        && domain
                            .as_deref()
                            .is_some_and(|value| !value.trim().is_empty())
                    {
                        Some(flow.domain.as_str())
                    } else {
                        None
                    };
                    persist_oauth_or_api_key(
                        &store,
                        &provider,
                        credential,
                        copilot_enterprise_domain_for_store,
                    )?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "authorized",
                            "credential": credential_kind,
                            "auth_file": store.path().display().to_string(),
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("status=authorized")?
                        || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                    {
                        return Ok(());
                    }
                }
                AuthMethod::OAuthBrowser => {
                    let (oauth_domain, client_id, client_secret) = match provider.as_str() {
                        "gitlab" => (
                            domain
                                .or_else(|| std::env::var("GITLAB_INSTANCE_URL").ok())
                                .unwrap_or_else(|| "gitlab.com".to_string()),
                            std::env::var("GITLAB_OAUTH_CLIENT_ID").ok(),
                            std::env::var("GITLAB_OAUTH_CLIENT_SECRET").ok(),
                        ),
                        "openai" => (
                            domain.unwrap_or_else(|| "auth.openai.com".to_string()),
                            None,
                            None,
                        ),
                        _ => {
                            let hint = oauth_login_hint(&provider).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "oauth login adapter for provider={provider} is not implemented yet"
                                )
                            })?;
                            if json_output {
                                let payload = serde_json::json!({
                                    "schema_version": 1,
                                    "provider": &provider,
                                    "method": selected_method.as_str(),
                                    "stage": "hint",
                                    "authorize_url": hint.authorize_url,
                                    "instructions": hint.instructions,
                                });
                                if !write_stdout_line(
                                    &serde_json::to_string(&payload)
                                        .context("failed to serialize auth login json")?,
                                )? {
                                    return Ok(());
                                }
                            } else if !write_stdout_line(&format!("provider={provider}"))?
                                || !write_stdout_line(&format!(
                                    "method={}",
                                    selected_method.as_str()
                                ))?
                                || !write_stdout_line(&format!(
                                    "authorize_url={}",
                                    hint.authorize_url
                                ))?
                                || !write_stdout_line(&format!(
                                    "instructions={}",
                                    hint.instructions
                                ))?
                            {
                                return Ok(());
                            }
                            return Ok(());
                        }
                    };

                    let flow = start_browser_oauth_flow(
                        &provider,
                        Some(&oauth_domain),
                        client_id.as_deref(),
                        oauth_port,
                    )?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "challenge",
                            "authorize_url": flow.authorize_url,
                            "redirect_uri": flow.redirect_uri,
                            "oauth_port": oauth_port,
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line(&format!("provider={provider}"))?
                        || !write_stdout_line(&format!("method={}", selected_method.as_str()))?
                        || !write_stdout_line(&format!("authorize_url={}", flow.authorize_url))?
                        || !write_stdout_line(&format!("redirect_uri={}", flow.redirect_uri))?
                        || !write_stdout_line(&format!("oauth_port={oauth_port}"))?
                    {
                        return Ok(());
                    }

                    if no_wait {
                        if json_output {
                            let payload = serde_json::json!({
                                "schema_version": 1,
                                "provider": &provider,
                                "method": selected_method.as_str(),
                                "stage": "awaiting_browser_callback",
                            });
                            if !write_stdout_line(
                                &serde_json::to_string(&payload)
                                    .context("failed to serialize auth login json")?,
                            )? {
                                return Ok(());
                            }
                        } else if !write_stdout_line("status=awaiting_browser_callback")? {
                            return Ok(());
                        }
                        return Ok(());
                    }

                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "waiting_for_callback",
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("status=waiting_for_callback")? {
                        return Ok(());
                    }
                    let timeout = Duration::from_secs(timeout_secs.max(1));
                    let credential =
                        complete_browser_oauth_flow(&flow, timeout, client_secret.as_deref())
                            .await?;
                    let credential_kind = login_credential_kind(&credential);
                    persist_oauth_or_api_key(&store, &provider, credential, None)?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "provider": &provider,
                            "method": selected_method.as_str(),
                            "stage": "authorized",
                            "credential": credential_kind,
                            "auth_file": store.path().display().to_string(),
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize auth login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("status=authorized")?
                        || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                    {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
}

async fn handle_mcp_command(
    command: McpCommand,
    json_output: bool,
    config: &ResolvedConfig,
) -> Result<()> {
    let store = AuthStore::open_default();
    let configured_servers = resolve_configured_mcp_servers(config)?;
    match command {
        McpCommand::List => {
            let stored_servers = store
                .providers()?
                .into_iter()
                .filter_map(|provider| provider.strip_prefix("mcp:").map(|name| name.to_string()))
                .collect::<Vec<_>>();
            let mut names = BTreeSet::new();
            for name in stored_servers {
                names.insert(name);
            }
            for name in configured_servers.keys() {
                names.insert(name.clone());
            }
            let servers = names.into_iter().collect::<Vec<_>>();

            if json_output {
                let rows = servers
                    .iter()
                    .map(|server| {
                        let key = mcp_store_key(server);
                        let configured = configured_servers.get(server);
                        Ok(serde_json::json!({
                            "name": server,
                            "credential": render_stored_credential(store.get(&key)?),
                            "configured": configured.is_some(),
                            "url": configured.and_then(|entry| entry.url.clone()),
                            "oauth_enabled": configured.map(|entry| entry.oauth_enabled()).unwrap_or(false),
                        }))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.list",
                    "auth_file": store.path().display().to_string(),
                    "servers": rows,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize mcp list json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_stdout_line(&format!("servers={}", servers.len()))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
            for server in servers {
                let key = mcp_store_key(&server);
                let configured = configured_servers.get(&server);
                let mut row = format!(
                    "name={server}\tcredential={}",
                    render_stored_credential(store.get(&key)?)
                );
                if let Some(configured) = configured {
                    row.push_str("\tconfigured=true");
                    if let Some(url) = &configured.url {
                        row.push_str(&format!("\turl={url}"));
                    }
                    row.push_str(&format!("\toauth_enabled={}", configured.oauth_enabled()));
                }
                if !write_stdout_line(&row)? {
                    return Ok(());
                }
            }
        }
        McpCommand::Status { name } => {
            let mut names: Vec<String> = if let Some(name) = name {
                vec![name]
            } else {
                configured_servers.keys().cloned().collect()
            };
            names.sort();

            let mut statuses = Vec::with_capacity(names.len());
            for name in names {
                let configured = configured_servers.get(&name).cloned();
                let key = mcp_store_key(&name);
                let credential = store.get(&key)?;
                let expires_at_unix = match credential.as_ref() {
                    Some(StoredCredential::OAuth {
                        expires_at_unix, ..
                    }) => *expires_at_unix,
                    _ => None,
                };
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|now| now.as_secs() as i64)
                    .unwrap_or(0);
                let expired = expires_at_unix
                    .and_then(|value| (value > 0).then_some(value))
                    .map(|value| value <= now_unix)
                    .unwrap_or(false);

                let (oauth_supported, metadata) = if let Some(configured) = configured.as_ref() {
                    if configured.oauth_enabled() {
                        if let Some(url) = configured.url.clone() {
                            let discovery = discover_mcp_oauth(&url).await?;
                            (
                                discovery.supported,
                                Some(serde_json::json!({
                                    "metadata_url": discovery.metadata_url,
                                    "authorization_endpoint": discovery.authorization_endpoint,
                                    "token_endpoint": discovery.token_endpoint,
                                })),
                            )
                        } else {
                            (false, None)
                        }
                    } else {
                        (false, None)
                    }
                } else {
                    (false, None)
                };

                statuses.push(serde_json::json!({
                    "name": name,
                    "configured": configured.is_some(),
                    "url": configured.as_ref().and_then(|entry| entry.url.clone()),
                    "oauth_enabled": configured.as_ref().map(|entry| entry.oauth_enabled()).unwrap_or(false),
                    "oauth_supported": oauth_supported,
                    "credential": render_stored_credential(credential),
                    "expired": expired,
                    "discovery": metadata,
                }));
            }

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.status",
                    "auth_file": store.path().display().to_string(),
                    "servers": statuses,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize mcp status json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_stdout_line(&format!("servers={}", statuses.len()))?
                || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
            for row in statuses {
                let name = row["name"].as_str().unwrap_or("<unknown>");
                let cred = row["credential"].as_str().unwrap_or("none");
                let configured = row["configured"].as_bool().unwrap_or(false);
                let url = row["url"].as_str().unwrap_or("-");
                let oauth_enabled = row["oauth_enabled"].as_bool().unwrap_or(false);
                let oauth_supported = row["oauth_supported"].as_bool().unwrap_or(false);
                let expired = row["expired"].as_bool().unwrap_or(false);
                let line = format!(
                    "name={name}\tconfigured={configured}\turl={url}\toauth_enabled={oauth_enabled}\toauth_supported={oauth_supported}\tcredential={cred}\texpired={expired}",
                );
                if !write_stdout_line(&line)? {
                    return Ok(());
                }
            }
        }
        McpCommand::Get { name } => {
            let key = mcp_store_key(&name);
            let configured = configured_servers.get(&name);
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.get",
                    "name": name,
                    "configured": configured.is_some(),
                    "url": configured.and_then(|entry| entry.url.clone()),
                    "oauth_enabled": configured.map(|entry| entry.oauth_enabled()).unwrap_or(false),
                    "credential": render_stored_credential(store.get(&key)?),
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload).context("failed to serialize mcp get json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_stdout_line(&format!("name={name}"))?
                || !write_stdout_line(&format!(
                    "credential={}",
                    render_stored_credential(store.get(&key)?)
                ))?
            {
                return Ok(());
            }
            if let Some(configured) = configured {
                if let Some(url) = &configured.url {
                    if !write_stdout_line(&format!("url={url}"))? {
                        return Ok(());
                    }
                }
                if !write_stdout_line(&format!("oauth_enabled={}", configured.oauth_enabled()))? {
                    return Ok(());
                }
            } else if !write_stdout_line("configured=false")? {
                return Ok(());
            }
        }
        McpCommand::Add {
            name,
            url,
            oauth,
            client_id,
            client_secret_env,
            scope,
        } => {
            let cwd = std::env::current_dir().context("failed to resolve current directory")?;
            let scope = parse_mcp_edit_scope(&scope)?;
            let oauth_enabled = match oauth.as_deref() {
                None => true,
                Some("on") => true,
                Some("off") => false,
                Some(other) => anyhow::bail!("invalid oauth toggle: {other}"),
            };
            let server = rustcode_core::config::McpServerConfig {
                url: Some(url.clone()),
                oauth: rustcode_core::config::McpOAuthConfig {
                    enabled: oauth_enabled,
                    client_id: client_id.clone(),
                    client_secret_env: client_secret_env.clone(),
                },
            };
            let path = edit_mcp_server(scope, &cwd, &name, &server)
                .map_err(|err| anyhow::anyhow!(err.to_string()))
                .context("failed to write MCP server config")?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.add",
                    "name": name,
                    "scope": render_mcp_edit_scope(scope),
                    "config_file": path.display().to_string(),
                    "url": url,
                    "oauth_enabled": oauth_enabled,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload).context("failed to serialize mcp add json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_stdout_line(&format!("added mcp server name={name}"))?
                || !write_stdout_line(&format!("scope={}", render_mcp_edit_scope(scope)))?
                || !write_stdout_line(&format!("config_file={}", path.display()))?
            {
                return Ok(());
            }
        }
        McpCommand::Remove { name, scope } => {
            let cwd = std::env::current_dir().context("failed to resolve current directory")?;
            let scope = parse_mcp_edit_scope(&scope)?;
            let (path, removed) = remove_mcp_server(scope, &cwd, &name)
                .map_err(|err| anyhow::anyhow!(err.to_string()))
                .context("failed to remove MCP server config")?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.remove",
                    "name": name,
                    "scope": render_mcp_edit_scope(scope),
                    "config_file": path.display().to_string(),
                    "removed": removed,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize mcp remove json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }

            if !write_stdout_line(&format!(
                "{} mcp server name={name}",
                if removed { "removed" } else { "no configured" }
            ))? || !write_stdout_line(&format!("config_file={}", path.display()))?
            {
                return Ok(());
            }
        }
        McpCommand::Login {
            name,
            from_env,
            method,
            scopes,
            url,
            oauth_port,
            no_wait,
            timeout_secs,
            client_id,
            client_secret_env,
        } => {
            let selected_method = resolve_mcp_login_method(method.as_deref())?;
            if let Some(from_env) = from_env {
                if selected_method == McpLoginMethod::OAuthBrowser {
                    anyhow::bail!("--from-env cannot be combined with --method oauth_browser");
                }
                let token = std::env::var(&from_env).with_context(|| {
                    format!(
                        "environment variable {from_env} is not set; cannot store MCP credential"
                    )
                })?;
                let key = mcp_store_key(&name);
                store.set_api_key(&key, &token)?;

                if json_output {
                    let payload = serde_json::json!({
                        "schema_version": 1,
                        "command": "mcp.login",
                        "name": name,
                        "method": selected_method.as_str(),
                        "stage": "authorized",
                        "source": "env",
                        "credential": "stored:api_key",
                        "scopes": scopes,
                        "url": url,
                        "auth_file": store.path().display().to_string(),
                    });
                    if !write_stdout_line(
                        &serde_json::to_string(&payload)
                            .context("failed to serialize mcp login json")?,
                    )? {
                        return Ok(());
                    }
                    return Ok(());
                }

                if !write_stdout_line(&format!("stored mcp credential for name={name}"))?
                    || !write_stdout_line(&format!("source=env:{from_env}"))?
                    || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                {
                    return Ok(());
                }
                return Ok(());
            }

            let configured_server = configured_servers.get(&name);
            let resolved_url = resolve_mcp_login_url(&name, url.as_deref(), configured_server)
                .context("failed to resolve MCP login URL")?;

            let discovery = discover_mcp_oauth(&resolved_url).await?;
            if !discovery.supported {
                anyhow::bail!(
                    "MCP server does not advertise OAuth endpoints; provide --from-env <ENV_VAR> or confirm server OAuth metadata"
                );
            }

            if json_output {
                let discovered = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.login",
                    "name": name,
                    "method": selected_method.as_str(),
                    "stage": "oauth_discovered",
                    "url": resolved_url,
                    "metadata_url": discovery.metadata_url,
                    "authorization_endpoint": discovery.authorization_endpoint,
                    "token_endpoint": discovery.token_endpoint,
                    "scopes": scopes,
                });
                if !write_stdout_line(
                    &serde_json::to_string(&discovered)
                        .context("failed to serialize mcp login json")?,
                )? {
                    return Ok(());
                }
            } else {
                if !write_stdout_line(&format!("name={name}"))?
                    || !write_stdout_line(&format!("method={}", selected_method.as_str()))?
                    || !write_stdout_line("stage=oauth_discovered")?
                    || !write_stdout_line(&format!("url={resolved_url}"))?
                    || !write_stdout_line(&format!(
                        "metadata_url={}",
                        discovery.metadata_url.as_deref().unwrap_or("<unknown>")
                    ))?
                    || !write_stdout_line(&format!(
                        "authorization_endpoint={}",
                        discovery
                            .authorization_endpoint
                            .as_deref()
                            .unwrap_or("<unknown>")
                    ))?
                    || !write_stdout_line(&format!(
                        "token_endpoint={}",
                        discovery.token_endpoint.as_deref().unwrap_or("<unknown>")
                    ))?
                    || (!scopes.is_empty()
                        && !write_stdout_line(&format!("scopes={}", scopes.join(",")))?)
                {
                    return Ok(());
                }
            }

            match selected_method {
                McpLoginMethod::TokenImport => {
                    if json_output {
                        let awaiting = serde_json::json!({
                            "schema_version": 1,
                            "command": "mcp.login",
                            "name": name,
                            "method": selected_method.as_str(),
                            "stage": "awaiting_token_import",
                            "usage": "rustcode mcp login <name> --from-env <ENV_VAR>",
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&awaiting)
                                .context("failed to serialize mcp login json")?,
                        )? {
                            return Ok(());
                        }
                        return Ok(());
                    }

                    if !write_stdout_line("stage=awaiting_token_import")?
                        || !write_stdout_line(
                            "usage=rustcode mcp login <name> --from-env <ENV_VAR>",
                        )?
                    {
                        return Ok(());
                    }
                }
                McpLoginMethod::OAuthBrowser => {
                    let oauth_client_id = resolve_mcp_oauth_client_id(
                        client_id.as_deref(),
                        configured_server.and_then(McpConfigEntry::oauth_client_id),
                    )?;
                    let oauth_client_secret = resolve_mcp_oauth_client_secret(
                        client_secret_env.as_deref(),
                        configured_server.and_then(McpConfigEntry::oauth_client_secret_env),
                    )?;
                    let flow = start_mcp_browser_oauth_flow(
                        &name,
                        &resolved_url,
                        &discovery,
                        &oauth_client_id,
                        oauth_port,
                        &scopes,
                    )?;

                    if json_output {
                        let challenge = serde_json::json!({
                            "schema_version": 1,
                            "command": "mcp.login",
                            "name": name,
                            "method": selected_method.as_str(),
                            "stage": "challenge",
                            "url": resolved_url,
                            "authorize_url": flow.authorize_url,
                            "redirect_uri": flow.redirect_uri,
                            "oauth_port": oauth_port,
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&challenge)
                                .context("failed to serialize mcp login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("stage=challenge")?
                        || !write_stdout_line(&format!("authorize_url={}", flow.authorize_url))?
                        || !write_stdout_line(&format!("redirect_uri={}", flow.redirect_uri))?
                        || !write_stdout_line(&format!("oauth_port={oauth_port}"))?
                    {
                        return Ok(());
                    }

                    if no_wait {
                        if json_output {
                            let awaiting = serde_json::json!({
                                "schema_version": 1,
                                "command": "mcp.login",
                                "name": name,
                                "method": selected_method.as_str(),
                                "stage": "awaiting_browser_callback",
                            });
                            if !write_stdout_line(
                                &serde_json::to_string(&awaiting)
                                    .context("failed to serialize mcp login json")?,
                            )? {
                                return Ok(());
                            }
                            return Ok(());
                        }
                        if !write_stdout_line("stage=awaiting_browser_callback")? {
                            return Ok(());
                        }
                        return Ok(());
                    }

                    if json_output {
                        let waiting = serde_json::json!({
                            "schema_version": 1,
                            "command": "mcp.login",
                            "name": name,
                            "method": selected_method.as_str(),
                            "stage": "waiting_for_callback",
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&waiting)
                                .context("failed to serialize mcp login json")?,
                        )? {
                            return Ok(());
                        }
                    } else if !write_stdout_line("stage=waiting_for_callback")? {
                        return Ok(());
                    }

                    let timeout = Duration::from_secs(timeout_secs.max(1));
                    let credential = complete_mcp_browser_oauth_flow(
                        &flow,
                        timeout,
                        oauth_client_secret.as_deref(),
                    )
                    .await?;
                    let key = mcp_store_key(&name);
                    let expires_at_unix = credential.expires_in_secs.map(|secs| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|now| now.as_secs().saturating_add(secs) as i64)
                            .unwrap_or(secs as i64)
                    });
                    store.set_oauth(
                        &key,
                        &credential.access_token,
                        credential.refresh_token.as_deref(),
                        expires_at_unix,
                        credential.account_id.as_deref(),
                    )?;
                    if json_output {
                        let payload = serde_json::json!({
                            "schema_version": 1,
                            "command": "mcp.login",
                            "name": name,
                            "method": selected_method.as_str(),
                            "stage": "authorized",
                            "credential": "stored:oauth",
                            "auth_file": store.path().display().to_string(),
                        });
                        if !write_stdout_line(
                            &serde_json::to_string(&payload)
                                .context("failed to serialize mcp login json")?,
                        )? {
                            return Ok(());
                        }
                        return Ok(());
                    }

                    if !write_stdout_line("stage=authorized")?
                        || !write_stdout_line("credential=stored:oauth")?
                        || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
                    {
                        return Ok(());
                    }
                }
            }
        }
        McpCommand::Logout { name } => {
            let key = mcp_store_key(&name);
            let removed = store.remove(&key)?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "mcp.logout",
                    "name": name,
                    "removed": removed,
                    "auth_file": store.path().display().to_string(),
                });
                if !write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize mcp logout json")?,
                )? {
                    return Ok(());
                }
                return Ok(());
            }
            if !write_stdout_line(&format!(
                "{} mcp credential for name={name}",
                if removed { "removed" } else { "no stored" }
            ))? || !write_stdout_line(&format!("auth_file={}", store.path().display()))?
            {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize)]
struct McpConfigEntry {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    oauth: Option<McpOAuthConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum McpOAuthConfig {
    Enabled(bool),
    Settings(McpOAuthSettings),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct McpOAuthSettings {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret_env: Option<String>,
}

impl McpConfigEntry {
    fn oauth_enabled(&self) -> bool {
        match &self.oauth {
            None => true,
            Some(McpOAuthConfig::Enabled(enabled)) => *enabled,
            Some(McpOAuthConfig::Settings(settings)) => settings.enabled.unwrap_or(true),
        }
    }

    fn oauth_client_id(&self) -> Option<&str> {
        match &self.oauth {
            Some(McpOAuthConfig::Settings(settings)) => settings.client_id.as_deref(),
            _ => None,
        }
    }

    fn oauth_client_secret_env(&self) -> Option<&str> {
        match &self.oauth {
            Some(McpOAuthConfig::Settings(settings)) => settings.client_secret_env.as_deref(),
            _ => None,
        }
    }
}

impl From<&CoreMcpServerConfig> for McpConfigEntry {
    fn from(value: &CoreMcpServerConfig) -> Self {
        let oauth = Some(McpOAuthConfig::Settings(McpOAuthSettings {
            enabled: Some(value.oauth.enabled),
            client_id: value.oauth.client_id.clone(),
            client_secret_env: value.oauth.client_secret_env.clone(),
        }));
        Self {
            url: value.url.clone(),
            oauth,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpLoginMethod {
    TokenImport,
    OAuthBrowser,
}

impl McpLoginMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::TokenImport => "token_import",
            Self::OAuthBrowser => "oauth_browser",
        }
    }
}

fn resolve_mcp_login_method(raw: Option<&str>) -> Result<McpLoginMethod> {
    match raw.unwrap_or("token_import") {
        "token_import" => Ok(McpLoginMethod::TokenImport),
        "oauth_browser" => Ok(McpLoginMethod::OAuthBrowser),
        other => anyhow::bail!("unsupported mcp login method: {other}"),
    }
}

fn resolve_mcp_login_url(
    name: &str,
    provided_url: Option<&str>,
    configured: Option<&McpConfigEntry>,
) -> Result<String> {
    if let Some(url) = provided_url {
        return Ok(url.to_string());
    }
    if let Some(configured) = configured {
        if !configured.oauth_enabled() {
            anyhow::bail!("configured MCP server has oauth disabled; provide --from-env");
        }
        if let Some(url) = &configured.url {
            return Ok(url.clone());
        }
        anyhow::bail!("configured MCP server `{name}` is missing `url`");
    }
    anyhow::bail!("MCP login requires either --from-env <ENV_VAR>, --url <MCP_URL>, or a configured MCP server URL")
}

fn resolve_mcp_oauth_client_id(
    cli_client_id: Option<&str>,
    configured_client_id: Option<&str>,
) -> Result<String> {
    if let Some(client_id) = cli_client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(client_id.to_string());
    }
    if let Some(client_id) = configured_client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(client_id.to_string());
    }
    if let Ok(client_id) = std::env::var("RUSTCODE_MCP_OAUTH_CLIENT_ID") {
        let trimmed = client_id.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    anyhow::bail!(
        "mcp oauth browser login requires --client-id, configured oauth.client_id, or RUSTCODE_MCP_OAUTH_CLIENT_ID"
    )
}

fn resolve_mcp_oauth_client_secret(
    cli_secret_env: Option<&str>,
    configured_secret_env: Option<&str>,
) -> Result<Option<String>> {
    if let Some(name) = cli_secret_env
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let value = std::env::var(name).with_context(|| {
            format!("environment variable {name} is not set for MCP client secret")
        })?;
        return Ok(Some(value));
    }
    if let Some(name) = configured_secret_env
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let value = std::env::var(name).with_context(|| {
            format!("environment variable {name} is not set for MCP client secret")
        })?;
        return Ok(Some(value));
    }
    if let Ok(value) = std::env::var("RUSTCODE_MCP_OAUTH_CLIENT_SECRET") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }
    Ok(None)
}

fn load_mcp_servers_config() -> Result<BTreeMap<String, McpConfigEntry>> {
    let path = match resolve_mcp_servers_path() {
        Some(path) => path,
        None => return Ok(BTreeMap::new()),
    };
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read MCP servers config {}", path.display()))?;
    serde_json::from_str::<BTreeMap<String, McpConfigEntry>>(&raw)
        .with_context(|| format!("failed to parse MCP servers config {}", path.display()))
}

fn parse_mcp_edit_scope(raw: &str) -> Result<ConfigEditScope> {
    match raw {
        "user" => Ok(ConfigEditScope::User),
        "project" => Ok(ConfigEditScope::Project),
        other => anyhow::bail!("invalid scope: {other} (expected user|project)"),
    }
}

fn render_mcp_edit_scope(scope: ConfigEditScope) -> &'static str {
    match scope {
        ConfigEditScope::User => "user",
        ConfigEditScope::Project => "project",
    }
}

fn resolve_configured_mcp_servers(
    config: &ResolvedConfig,
) -> Result<BTreeMap<String, McpConfigEntry>> {
    let mut merged = load_mcp_servers_config()?;
    for (name, server) in &config.mcp_servers {
        merged.insert(name.clone(), McpConfigEntry::from(server));
    }
    Ok(merged)
}

fn resolve_mcp_servers_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUSTCODE_MCP_SERVERS_PATH") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
        return None;
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        let candidate = PathBuf::from(path).join("rustcode/mcp_servers.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(path) = std::env::var("HOME") {
        let candidate = PathBuf::from(path).join(".config/rustcode/mcp_servers.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn mcp_store_key(name: &str) -> String {
    format!("mcp:{name}")
}

fn render_stored_credential(credential: Option<StoredCredential>) -> &'static str {
    match credential {
        Some(StoredCredential::ApiKey { .. }) => "stored:api_key",
        Some(StoredCredential::OAuth { .. }) => "stored:oauth",
        None => "none",
    }
}

fn login_credential_kind(credential: &rustcode_auth::DeviceCodeFlowCredential) -> &'static str {
    if credential.refresh_token.is_some() || credential.expires_in_secs.is_some() {
        "stored:oauth"
    } else {
        "stored:api_key"
    }
}

fn classify_auth_error(err: &anyhow::Error) -> &'static str {
    let msg = err.to_string().to_ascii_lowercase();
    if msg.contains("network")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("connection")
        || msg.contains("failed to bind")
    {
        return "network";
    }
    if msg.contains("oauth polling failed")
        || msg.contains("authorization failed")
        || msg.contains("invalid oauth")
    {
        return "provider";
    }
    "validation"
}

fn classify_mcp_error(err: &anyhow::Error) -> &'static str {
    let msg = err.to_string().to_ascii_lowercase();
    if msg.contains("requires --client-id")
        || msg.contains("cannot be combined")
        || msg.contains("requires either --from-env")
        || msg.contains("failed to resolve mcp login url")
    {
        return "validation";
    }
    if msg.contains("network")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("connection")
    {
        return "network";
    }
    if msg.contains("oauth")
        || msg.contains("authorization_endpoint")
        || msg.contains("token_endpoint")
        || msg.contains("does not advertise")
    {
        return "provider";
    }
    "validation"
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
    let index = match load_models_index() {
        Ok(index) => Some(index),
        Err(_) => None,
    };
    let index_unavailable = index.is_none();

    if let Some(provider) = provider_filter {
        if !config.provider_allowed(provider) {
            anyhow::bail!("provider is disabled by config: {provider}");
        }
        let entry = index.as_ref().and_then(|index| index.get(provider));
        let diagnostics = diagnose_provider(config, Some(provider))
            .with_context(|| format!("failed to diagnose provider {provider}"))?;

        let mut model_ids: Vec<String> = entry
            .map(|entry| {
                entry
                    .models
                    .keys()
                    .map(|id| format!("{provider}/{id}"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        model_ids.sort();

        let provider_name = entry
            .and_then(|entry| entry.name.clone())
            .unwrap_or_else(|| provider.to_string());
        let index_warning = if index_unavailable {
            Some("models index unavailable; diagnostics only".to_string())
        } else if entry.is_some() {
            None
        } else {
            Some("provider not found in models index; diagnostics only".to_string())
        };

        if json_output {
            let payload = serde_json::json!({
                "schema_version": 1,
                "provider": {
                    "id": provider,
                    "name": provider_name,
                    "protocol": render_protocol(&diagnostics.protocol),
                    "base_url": diagnostics.base_url,
                    "endpoint": diagnostics.endpoint,
                    "requires_api_key": diagnostics.requires_api_key,
                    "api_key_source": render_api_key_source(&diagnostics.api_key_source),
                    "api_key_env_candidates": diagnostics.api_key_env_candidates,
                    "missing": diagnostics.missing,
                    "policy_score": diagnostics.policy_score,
                    "policy_selected": diagnostics.policy_selected,
                    "policy_available": diagnostics.policy_available,
                    "models": model_ids,
                    "warning": index_warning,
                }
            });
            if !write_stdout_line(
                &serde_json::to_string(&payload).context("failed to serialize models json")?,
            )? {
                return Ok(());
            }
            return Ok(());
        }

        if let Some(warning) = &index_warning {
            if !write_stdout_line(&format!("warning={warning}"))? {
                return Ok(());
            }
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
            || !write_stdout_line(&format!(
                "policy_score={}",
                diagnostics
                    .policy_score
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<n/a>".to_string())
            ))?
            || !write_stdout_line(&format!("policy_selected={}", diagnostics.policy_selected))?
            || !write_stdout_line(&format!(
                "policy_available={}",
                diagnostics
                    .policy_available
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<n/a>".to_string())
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

    let index = index.unwrap_or_default();
    let mut providers: Vec<_> = index
        .into_iter()
        .filter(|(provider_id, _)| config.provider_allowed(provider_id))
        .collect();
    providers.sort_by(|a, b| a.0.cmp(&b.0));

    if providers.is_empty() && index_unavailable {
        providers = builtin_provider_ids()
            .iter()
            .filter(|provider_id| config.provider_allowed(provider_id))
            .map(|provider_id| {
                (
                    provider_id.to_string(),
                    ModelsProvider {
                        name: None,
                        models: BTreeMap::new(),
                    },
                )
            })
            .collect();
        providers.sort_by(|a, b| a.0.cmp(&b.0));
    }

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
                "policy_score": diagnostics.policy_score,
                "policy_selected": diagnostics.policy_selected,
                "policy_available": diagnostics.policy_available,
            }));
        }
        let payload = serde_json::json!({
            "schema_version": 1,
            "warning": if index_unavailable { Some("models index unavailable; showing builtin presets only".to_string()) } else { None },
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
            "{provider_id}\tmodels={}\tname={display_name}\tprotocol={}\tendpoint={}\tapi_key_source={}\tmissing={missing}\tpolicy_score={}\tpolicy_selected={}\tpolicy_available={}",
            entry.models.len(),
            render_protocol(&diagnostics.protocol),
            diagnostics.endpoint.as_deref().unwrap_or("<unset>"),
            render_api_key_source(&diagnostics.api_key_source),
            diagnostics
                .policy_score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<n/a>".to_string()),
            diagnostics.policy_selected,
            diagnostics
                .policy_available
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<n/a>".to_string()),
        ))? {
            return Ok(());
        }
    }
    Ok(())
}

fn resolve_login_method(method: Option<&str>, available: &[AuthMethod]) -> Result<AuthMethod> {
    if let Some(raw) = method {
        let requested = match raw {
            "api_key" => AuthMethod::ApiKey,
            "oauth_device_code" => AuthMethod::OAuthDeviceCode,
            "oauth_browser" => AuthMethod::OAuthBrowser,
            _ => anyhow::bail!("unknown auth method: {raw}"),
        };
        if available.iter().any(|item| item == &requested) {
            return Ok(requested);
        }
        anyhow::bail!(
            "requested method={raw} is unavailable; supported methods are: {}",
            available
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    available
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("provider does not advertise any authentication methods"))
}

fn resolve_login_method_with_context(
    provider: &str,
    method: Option<&str>,
    available: &[AuthMethod],
    from_env_supplied: bool,
) -> Result<AuthMethod> {
    if from_env_supplied && method.is_none() {
        if available.contains(&AuthMethod::ApiKey) {
            return Ok(AuthMethod::ApiKey);
        }
        anyhow::bail!("provider={provider} does not support api_key login via --from-env");
    }

    if method.is_some() {
        return resolve_login_method(method, available);
    }

    if available.len() > 1 && is_interactive_terminal() {
        return prompt_for_login_method(provider, available);
    }

    resolve_login_method(None, available)
}

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

#[derive(Debug, Clone, Copy)]
struct StdioToolApprover;

#[async_trait::async_trait]
impl rustcode_core::ToolApprover for StdioToolApprover {
    async fn approve(
        &self,
        request: rustcode_core::ToolApprovalRequest,
    ) -> std::result::Result<bool, rustcode_core::ExecutionError> {
        tokio::task::spawn_blocking(move || {
            let mut stderr = std::io::stderr().lock();
            writeln!(stderr, "approval required: {}", request.reason)
                .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?;
            writeln!(
                stderr,
                "tool={} permission={} pattern={}",
                request.tool, request.permission, request.pattern
            )
            .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?;
            write!(stderr, "approve? [y/N]: ")
                .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?;
            stderr
                .flush()
                .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?;

            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?;
            let answer = input.trim();
            Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
        })
        .await
        .map_err(|err| rustcode_core::ExecutionError::Executor(err.to_string()))?
    }
}

fn prompt_for_login_method(provider: &str, available: &[AuthMethod]) -> Result<AuthMethod> {
    let mut stderr = std::io::stderr().lock();
    writeln!(
        stderr,
        "provider={provider} supports multiple auth methods:"
    )
    .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    for (idx, method) in available.iter().enumerate() {
        writeln!(stderr, "  {}. {}", idx + 1, method.as_str())
            .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    }
    write!(
        stderr,
        "select auth method [1-{}] (default 1): ",
        available.len()
    )
    .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    stderr
        .flush()
        .map_err(|err| anyhow::anyhow!("stderr flush failed: {err}"))?;

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|err| anyhow::anyhow!("failed to read auth method selection: {err}"))?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(available[0]);
    }
    let index: usize = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid auth method selection: {trimmed}"))?;
    let zero_based = index
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("invalid auth method selection: {trimmed}"))?;
    available
        .get(zero_based)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("invalid auth method selection: {trimmed}"))
}

fn prompt_for_api_key(provider: &str) -> Result<String> {
    if !is_interactive_terminal() {
        anyhow::bail!(
            "method=api_key requires --from-env in non-interactive mode: `rustcode auth login {provider} --from-env <ENV_VAR>`"
        );
    }

    let mut stderr = std::io::stderr().lock();
    write!(stderr, "enter API key for provider={provider}: ")
        .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    stderr
        .flush()
        .map_err(|err| anyhow::anyhow!("stderr flush failed: {err}"))?;

    let key = match rpassword::read_password() {
        Ok(value) => value,
        Err(err) => {
            // Some sandboxed PTY environments deny the ioctl used by rpassword.
            // Fall back to a normal line read rather than failing the login flow.
            writeln!(
                stderr,
                "\nWARN: could not disable terminal echo ({err}); falling back to plaintext input"
            )
            .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
            let mut value = String::new();
            std::io::stdin()
                .read_line(&mut value)
                .map_err(|err| anyhow::anyhow!("failed to read api key input: {err}"))?;
            value
        }
    };
    let key = key.trim().to_string();
    if key.is_empty() {
        anyhow::bail!("api key input must not be empty");
    }
    Ok(key)
}

fn prompt_for_login_provider() -> Result<String> {
    if !is_interactive_terminal() {
        anyhow::bail!("cannot prompt for provider in non-interactive mode");
    }

    // Keep this list intentionally small (OpenCode-style "top providers") and offer "other"
    // to avoid dumping the full provider universe to the user.
    const PROVIDERS: [(&str, &str, Option<&str>); 7] = [
        ("opencode", "OpenCode Zen", Some("recommended")),
        ("anthropic", "Anthropic", Some("Claude Max or API key")),
        ("github-copilot", "GitHub Copilot", None),
        ("openai", "OpenAI", Some("ChatGPT Plus/Pro or API key")),
        ("google", "Google", None),
        ("openrouter", "OpenRouter", None),
        ("vercel", "Vercel AI Gateway", None),
    ];

    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "add credential")
        .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    writeln!(stderr, "select provider:")
        .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    for (idx, (_id, label, hint)) in PROVIDERS.iter().enumerate() {
        if let Some(hint) = hint {
            writeln!(stderr, "  {}. {} ({})", idx + 1, label, hint)
                .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
        } else {
            writeln!(stderr, "  {}. {}", idx + 1, label)
                .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
        }
    }
    writeln!(stderr, "  {}. Other", PROVIDERS.len() + 1)
        .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    write!(
        stderr,
        "select provider [1-{}] (default 1): ",
        PROVIDERS.len() + 1
    )
    .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    stderr
        .flush()
        .map_err(|err| anyhow::anyhow!("stderr flush failed: {err}"))?;

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|err| anyhow::anyhow!("failed to read provider selection: {err}"))?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(PROVIDERS[0].0.to_string());
    }

    if let Ok(index) = trimmed.parse::<usize>() {
        let zero_based = index
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("invalid provider selection: {trimmed}"))?;
        if zero_based < PROVIDERS.len() {
            return Ok(PROVIDERS[zero_based].0.to_string());
        }
        if zero_based == PROVIDERS.len() {
            return prompt_for_custom_provider_id();
        }
        anyhow::bail!("invalid provider selection: {trimmed}");
    }

    // Allow directly typing a provider id (useful for scripts in a tty).
    validate_provider_id(trimmed)?;
    Ok(trimmed.to_string())
}

fn prompt_for_custom_provider_id() -> Result<String> {
    let mut stderr = std::io::stderr().lock();
    write!(stderr, "enter provider id (a-z, 0-9, hyphens): ")
        .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
    stderr
        .flush()
        .map_err(|err| anyhow::anyhow!("stderr flush failed: {err}"))?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|err| anyhow::anyhow!("failed to read provider id: {err}"))?;
    let trimmed = input.trim();
    validate_provider_id(trimmed)?;
    Ok(trimmed.to_string())
}

fn validate_provider_id(value: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("provider id must not be empty");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        anyhow::bail!("provider id must match [a-z0-9-]+");
    }
    Ok(())
}

fn print_provider_api_key_hint(provider: &str) -> Result<()> {
    if !is_interactive_terminal() {
        return Ok(());
    }
    let mut stderr = std::io::stderr().lock();
    match provider {
        "opencode" => {
            writeln!(
                stderr,
                "hint: create an api key at https://opencode.ai/auth"
            )
            .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
        }
        "vercel" => {
            writeln!(
                stderr,
                "hint: create an api key at https://vercel.link/ai-gateway-token"
            )
            .map_err(|err| anyhow::anyhow!("stderr write failed: {err}"))?;
        }
        _ => {}
    }
    Ok(())
}

fn persist_oauth_or_api_key(
    store: &AuthStore,
    provider: &str,
    credential: rustcode_auth::DeviceCodeFlowCredential,
    domain: Option<&str>,
) -> Result<()> {
    if credential.refresh_token.is_some() || credential.expires_in_secs.is_some() {
        let expires_at_unix = credential.expires_in_secs.map(|secs| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| now.as_secs().saturating_add(secs) as i64)
                .unwrap_or(secs as i64)
        });
        store.set_oauth(
            provider,
            &credential.access_token,
            credential.refresh_token.as_deref(),
            expires_at_unix,
            credential.account_id.as_deref(),
        )?;
    } else {
        if provider == "github-copilot-enterprise" {
            store.set_api_key_with_domain(provider, &credential.access_token, domain)?;
        } else {
            store.set_api_key(provider, &credential.access_token)?;
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

fn write_stdout_raw(text: &str) -> Result<bool> {
    let mut stdout = std::io::stdout().lock();
    match write!(stdout, "{text}") {
        Ok(()) => {
            stdout
                .flush()
                .map_err(|err| anyhow::anyhow!("stdout flush failed: {err}"))?;
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(anyhow::anyhow!("stdout write failed: {err}")),
    }
}

#[derive(Debug)]
struct AuthMethodRow {
    provider_id: String,
    display_name: String,
    methods: Vec<String>,
}

fn resolve_auth_method_rows() -> (Vec<AuthMethodRow>, Option<String>) {
    match load_models_index() {
        Ok(index) => {
            let mut rows = index
                .into_iter()
                .map(|(provider_id, provider)| AuthMethodRow {
                    display_name: provider.name.unwrap_or_else(|| provider_id.clone()),
                    methods: methods_for_provider(&provider_id)
                        .into_iter()
                        .map(|method| method.as_str().to_string())
                        .collect(),
                    provider_id,
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| {
                auth_login_priority(&a.provider_id)
                    .cmp(&auth_login_priority(&b.provider_id))
                    .then_with(|| a.display_name.cmp(&b.display_name))
            });
            (rows, None)
        }
        Err(err) => {
            let mut rows = known_oauth_providers()
                .iter()
                .map(|provider| AuthMethodRow {
                    provider_id: (*provider).to_string(),
                    display_name: (*provider).to_string(),
                    methods: methods_for_provider(provider)
                        .into_iter()
                        .map(|method| method.as_str().to_string())
                        .collect(),
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
            (rows, Some(err.to_string()))
        }
    }
}

fn list_auth_login_providers(json_output: bool) -> Result<()> {
    let (rows, warning) = resolve_auth_method_rows();
    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "warning": warning,
            "usage": {
                "api_key": "rustcode auth login <provider> --from-env <ENV_VAR>",
                "oauth": "rustcode auth login <provider> --method <oauth_device_code|oauth_browser>",
            },
            "providers": rows
                .iter()
                .map(|row| serde_json::json!({
                    "id": row.provider_id,
                    "name": row.display_name,
                    "methods": row.methods,
                }))
                .collect::<Vec<_>>(),
        });
        if !write_stdout_line(
            &serde_json::to_string(&payload).context("failed to serialize auth login json")?,
        )? {
            return Ok(());
        }
        return Ok(());
    }

    if let Some(warning) = warning {
        if !write_stdout_line(&format!("warning={warning}"))? {
            return Ok(());
        }
    }

    if !write_stdout_line("usage_api_key=rustcode auth login <provider> --from-env <ENV_VAR>")?
        || !write_stdout_line(
            "usage_oauth=rustcode auth login <provider> --method <oauth_device_code|oauth_browser>",
        )?
        || !write_stdout_line(&format!("providers={}", rows.len()))?
    {
        return Ok(());
    }
    for row in rows {
        if !write_stdout_line(&format!(
            "provider={}\tname={}\tmethods={}",
            row.provider_id,
            row.display_name,
            row.methods.join("|")
        ))? {
            return Ok(());
        }
    }
    Ok(())
}

fn list_auth_methods(json_output: bool) -> Result<()> {
    let (rows, warning) = resolve_auth_method_rows();
    if json_output {
        let payload = serde_json::json!({
            "schema_version": 1,
            "warning": warning,
            "providers": rows
                .iter()
                .map(|row| serde_json::json!({
                    "id": row.provider_id,
                    "name": row.display_name,
                    "methods": row.methods,
                }))
                .collect::<Vec<_>>(),
        });
        if !write_stdout_line(
            &serde_json::to_string(&payload).context("failed to serialize auth methods json")?,
        )? {
            return Ok(());
        }
        return Ok(());
    }

    if let Some(warning) = warning {
        if !write_stdout_line(&format!("warning={warning}"))? {
            return Ok(());
        }
    }

    if !write_stdout_line(&format!("providers={}", rows.len()))? {
        return Ok(());
    }
    for row in rows {
        if !write_stdout_line(&format!(
            "provider={}\tname={}\tmethods={}",
            row.provider_id,
            row.display_name,
            row.methods.join("|")
        ))? {
            return Ok(());
        }
    }
    Ok(())
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
        ProviderProtocolName::GoogleGenerativeAi => "google_generative_ai",
        ProviderProtocolName::VercelAiGateway => "vercel_ai_gateway",
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

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn resolve_session(
    store: &SessionStore,
    config: &ResolvedConfig,
    continue_session: bool,
    session_id: Option<&str>,
    fork: bool,
    title: Option<String>,
) -> Result<(rustcode_core::SessionInfo, Vec<rustcode_core::StoredMessage>)> {
    if fork && !continue_session && session_id.is_none() {
        anyhow::bail!("--fork requires --continue or --session <SESSION_ID>");
    }

    let base = if continue_session {
        let sessions = store
            .list_sessions()
            .context("failed to list sessions")?;
        sessions
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no sessions exist to continue"))?
    } else if let Some(session_id) = session_id {
        store
            .get_session(session_id)
            .with_context(|| format!("failed to load session {session_id}"))?
    } else {
        let cwd = std::env::current_dir().context("failed to resolve cwd")?;
        store
            .create_session(
                title.clone(),
                None,
                &cwd,
                &config.workspace_root,
                &config.model,
            )
            .context("failed to create session")?
    };

    let info = if fork {
        store
            .fork_session(&base.id, title)
            .with_context(|| format!("failed to fork session {}", base.id))?
    } else {
        base
    };

    let history = store
        .load_messages(&info.id)
        .with_context(|| format!("failed to load transcript for session {}", info.id))?;

    Ok((info, history))
}

fn handle_session_command(
    command: SessionCommand,
    json_output: bool,
    config: &ResolvedConfig,
) -> Result<()> {
    let store = SessionStore::open_default();
    match command {
        SessionCommand::List => {
            let sessions = store.list_sessions().context("failed to list sessions")?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.list",
                    "sessions": sessions,
                    "sessions_root": store.root().display().to_string(),
                });
                write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize session list json")?,
                )?;
                return Ok(());
            }

            write_stdout_line(&format!("sessions={}", sessions.len()))?;
            write_stdout_line(&format!("sessions_root={}", store.root().display()))?;
            for session in sessions {
                let title = session.title.as_deref().unwrap_or("-");
                let parent = session.parent_id.as_deref().unwrap_or("-");
                write_stdout_line(&format!(
                    "id={}\tupdated_at={}\ttitle={}\tparent_id={}",
                    session.id, session.updated_at_unix_ms, title, parent
                ))?;
            }
        }
        SessionCommand::New { title } => {
            let cwd = std::env::current_dir().context("failed to resolve cwd")?;
            let session = store
                .create_session(
                    title,
                    None,
                    &cwd,
                    &config.workspace_root,
                    &config.model,
                )
                .context("failed to create session")?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.new",
                    "session": session,
                });
                write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize session new json")?,
                )?;
                return Ok(());
            }
            write_stdout_line(&format!("id={}", session.id))?;
        }
        SessionCommand::Fork { session_id, title } => {
            let forked = store
                .fork_session(&session_id, title)
                .with_context(|| format!("failed to fork session {session_id}"))?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.fork",
                    "session": forked,
                });
                write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize session fork json")?,
                )?;
                return Ok(());
            }
            write_stdout_line(&format!("id={}", forked.id))?;
        }
        SessionCommand::Show { session_id } => {
            let session = store
                .get_session(&session_id)
                .with_context(|| format!("failed to load session {session_id}"))?;
            let messages = store
                .load_messages(&session_id)
                .with_context(|| format!("failed to load transcript for session {session_id}"))?;

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "session.show",
                    "session": session,
                    "messages": messages,
                });
                write_stdout_line(
                    &serde_json::to_string(&payload)
                        .context("failed to serialize session show json")?,
                )?;
                return Ok(());
            }

            write_stdout_line(&format!("id={}", session.id))?;
            write_stdout_line(&format!(
                "title={}",
                session.title.as_deref().unwrap_or("-")
            ))?;
            write_stdout_line(&format!("parent_id={}", session.parent_id.as_deref().unwrap_or("-")))?;
            write_stdout_line(&format!("created_at={}", session.created_at_unix_ms))?;
            write_stdout_line(&format!("updated_at={}", session.updated_at_unix_ms))?;
            write_stdout_line(&format!("cwd={}", session.cwd))?;
            write_stdout_line(&format!("workspace_root={}", session.workspace_root))?;
            write_stdout_line(&format!("model={}", session.model))?;
            write_stdout_line(&format!("messages={}", messages.len()))?;
        }
    }
    Ok(())
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
