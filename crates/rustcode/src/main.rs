use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{
    Parser,
    error::ErrorKind::{DisplayHelpOnMissingArgumentOrSubcommand, MissingSubcommand},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use rustcode_core::context::{CommandContext, SessionMeta};
use rustcode_core::error::ExecutionError;
use rustcode_core::event::EventPayload;
use rustcode_core::ports::{CommandExecutor, ToolApprover, TranscriptRecorder};
use rustcode_engine::{ChannelPublisher, Engine, WorkspacePermissionPolicy};
use rustcode_io::LocalIo;
use rustcode_llm::build_client;
use rustcode_plugins::PluginRegistry;
use rustcode_state::{FileTranscriptRecorder, SessionStore};

mod agent_cmds;
mod auth;
mod cli;
mod command_dispatch;
mod github;
mod mcp;
mod models;
mod render;
mod run_cmds;
mod session_cmds;
mod tui_cmds;
mod utils;
mod worktree_cmds;

use agent_cmds::StdioToolApprover;
use auth::{classify_auth_error, handle_auth_command};
use cli::{AuthCommand, Cli, McpCommand, TopCommand};
use command_dispatch::build_engine_command;
use github::{handle_github_command, handle_pr_command};
use mcp::{build_mcp_registry, classify_mcp_error, handle_mcp_command};
use models::handle_models_command;
use render::{render_event, OutputFormat};
use run_cmds::run_attached;
use session_cmds::{
    handle_export_command, handle_import_command, handle_session_command, resolve_session,
};
use tui_cmds::{handle_tui_command, handle_tui_default};
use utils::{
    is_interactive_terminal, load_effective_config, now_unix_ms, wait_for_shutdown_signal,
    write_stdout_line, write_stdout_raw,
};
use worktree_cmds::handle_worktree_command;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI args first so we can choose the right logging mode before
    // claiming the global tracing subscriber slot.
    let raw_args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(raw_args.clone()) {
        Ok(cli) => cli,
        Err(err)
            if matches!(
                err.kind(),
                MissingSubcommand | DisplayHelpOnMissingArgumentOrSubcommand
            ) =>
        {
            let mut injected = raw_args;
            injected.push(std::ffi::OsString::from("tui"));
            Cli::parse_from(injected)
        }
        Err(err) => err.exit(),
    };

    // In TUI mode, tracing output must be suppressed: log messages written to
    // stderr bleed through ratatui's alternate-screen buffer and corrupt the UI.
    // For non-TUI commands, keep the usual stderr + optional OTel setup.
    let is_tui = matches!(&cli.command, TopCommand::Tui(_));
    let _otel_guard = if is_tui {
        rustcode_logging::init_silent();
        None
    } else {
        let otel_endpoint = std::env::var("RUSTCODE_OTEL_ENDPOINT").ok();
        rustcode_logging::init_with_otel(otel_endpoint.as_deref())
    };

    // Chain a panic hook that emits a tracing error before propagating.
    // This ensures panics are captured in log files / OTel when available.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        prev_hook(info);
    }));

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
        return match handle_mcp_command(command.clone(), cli.json, cli.trust_project_config).await {
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
        };
    }
    if let TopCommand::Models { provider } = &cli.command {
        return handle_models_command(provider.clone(), cli.json, &cli);
    }
    if let TopCommand::Session { command } = &cli.command {
        return handle_session_command(command.clone(), cli.json);
    }
    if let TopCommand::Worktree { command } = &cli.command {
        return handle_worktree_command(command, cli.json);
    }

    if let TopCommand::Export { session_id } = &cli.command {
        return handle_export_command(session_id.as_deref());
    }
    if let TopCommand::Import { file } = &cli.command {
        return handle_import_command(file);
    }

    if let TopCommand::GitHub { command } = &cli.command {
        return handle_github_command(command, cli.json);
    }
    if let TopCommand::Pr { command } = &cli.command {
        return handle_pr_command(command.clone(), cli.json);
    }

    if let TopCommand::Tui(tui_opts) = &cli.command {
        if let Some(tui_cmd) = &tui_opts.command {
            return handle_tui_command(tui_cmd.clone(), &cli).await;
        }
        return handle_tui_default(
            &cli,
            tui_opts.fork,
            tui_opts.continue_session,
            tui_opts.session.clone(),
            tui_opts.prompt.clone(),
            tui_opts.title.clone(),
        )
        .await;
    }

    let requires_llm = matches!(
        &cli.command,
        TopCommand::Agent { .. } | TopCommand::Serve { .. } | TopCommand::Run { attach: None, .. }
    );
    let output_format = OutputFormat::from_json_flag(cli.json);
    let event_debug = cli.event_debug;

    let config = load_effective_config(&cli)?;

    if let TopCommand::Run {
        prompt,
        attach: Some(url),
        session,
        fork,
        title,
        ..
    } = &cli.command
    {
        if !config.allow_network {
            tracing::warn!("run --attach with network disabled; remote calls will fail");
        }
        if *fork || title.is_some() {
            anyhow::bail!("run --attach does not support local session flags like --fork/--title");
        }
        return run_attached(
            url,
            Some(prompt.as_str()),
            session.as_deref(),
            output_format,
            event_debug,
        )
        .await;
    }

    let session_store = SessionStore::open_default();
    let (session_info, run_history, agent_history) = match &cli.command {
        TopCommand::Run {
            prompt: _,
            attach: None,
            continue_session,
            session,
            fork,
            title,
        } => {
            let id = resolve_session(
                &session_store,
                *continue_session,
                session.clone(),
                if *fork {
                    session.clone().or_else(|| {
                        let mut items: Vec<_> =
                            session_store.list_sessions().ok()?.into_iter().collect();
                        items.sort_by_key(|m| std::cmp::Reverse(m.created_at_unix_ms));
                        items.into_iter().next().map(|m| m.id)
                    })
                } else {
                    None
                },
                title.clone(),
                &config.model,
            )?;
            let meta = session_store.get_session(&id)?;
            let messages = session_store.load_messages(&id)?;
            (Some(meta), messages, Vec::new())
        }
        TopCommand::Agent {
            prompt: _,
            continue_session,
            session,
            fork,
            title,
            ..
        } => {
            let id = resolve_session(
                &session_store,
                *continue_session,
                session.clone(),
                if *fork {
                    session.clone().or_else(|| {
                        let mut items: Vec<_> =
                            session_store.list_sessions().ok()?.into_iter().collect();
                        items.sort_by_key(|m| std::cmp::Reverse(m.created_at_unix_ms));
                        items.into_iter().next().map(|m| m.id)
                    })
                } else {
                    None
                },
                title.clone(),
                &config.model,
            )?;
            let meta = session_store.get_session(&id)?;
            let messages = session_store.load_messages(&id)?;
            (Some(meta), Vec::new(), messages)
        }
        _ => (None, Vec::new(), Vec::new()),
    };

    let session_id = session_info
        .as_ref()
        .map_or_else(|| "session-1".to_string(), |info| info.id.clone());

    let run_user_prompt = match &cli.command {
        TopCommand::Run {
            prompt,
            attach: None,
            ..
        } => Some(prompt.clone()),
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

    let recorder: Option<Arc<dyn TranscriptRecorder>> =
        if matches!(&cli.command, TopCommand::Agent { .. }) {
            Some(Arc::new(FileTranscriptRecorder::new(session_store.clone())))
        } else {
            None
        };

    let approver: Option<Arc<dyn ToolApprover>> = if matches!(
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

    let llm_client = if requires_llm && config.allow_network {
        match build_client(&config) {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!("LLM provider init failed, using null client: {err}");
                Arc::new(rustcode_llm::NullLlmClient)
            }
        }
    } else {
        if requires_llm && !config.allow_network {
            tracing::info!("network access disabled — using null LLM client");
        }
        Arc::new(rustcode_llm::NullLlmClient)
    };

    let mcp_registry = if matches!(
        &cli.command,
        TopCommand::Agent { .. } | TopCommand::Run { attach: None, .. }
    ) && config.allow_network
    {
        Some(build_mcp_registry(&config).await?)
    } else {
        None
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
    let mut engine = Engine::new(
        llm_client,
        io.clone(),
        io,
        Arc::new(WorkspacePermissionPolicy),
        PluginRegistry::default(),
        recorder,
        approver,
    );
    if let Some(registry) = mcp_registry {
        engine = engine.with_mcp(registry);
    }
    let command = build_engine_command(cli.command, &run_history, agent_history)?;

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

    {
        let mut streamed_text_open = false;
        let mut run_output_capture = String::new();
        while let Some(event) = event_rx.recv().await {
            if matches!(output_format, OutputFormat::Human) && !event_debug {
                match &event.payload {
                    EventPayload::OutputChunk { text } => {
                        if run_session_id.is_some()
                            && matches!(event.scope, rustcode_core::event::EventScope::Command)
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
                let user = rustcode_core::session::StoredMessage {
                    id: store.new_message_id(),
                    role: rustcode_core::session::MessageRole::User,
                    created_at_unix_ms: now_unix_ms(),
                    content: serde_json::Value::String(user_prompt),
                    reasoning: None,
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                };
                let assistant_text = run_output_capture.trim_end().to_string();
                let assistant = rustcode_core::session::StoredMessage {
                    id: store.new_message_id(),
                    role: rustcode_core::session::MessageRole::Assistant,
                    created_at_unix_ms: now_unix_ms(),
                    content: serde_json::Value::String(assistant_text),
                    reasoning: None,
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
