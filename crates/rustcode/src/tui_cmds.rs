use crate::cli::{Cli, TuiCommand};
use crate::mcp::build_mcp_registry;
use crate::run_cmds::RemoteRunExecutor;
use crate::utils::{is_interactive_terminal, load_effective_config};
use anyhow::{Context, Result};
use rustcode_core::config::ResolvedConfig;
use rustcode_engine::{Engine, WorkspacePermissionPolicy};
use rustcode_io::LocalIo;
use rustcode_llm::build_client;
use rustcode_plugins::PluginRegistry;
use rustcode_state::{FileTranscriptRecorder, SessionStore};
use std::sync::Arc;

pub async fn handle_tui_command(tui: TuiCommand, cli: &Cli) -> Result<()> {
    if !is_interactive_terminal() {
        return Ok(());
    }

    let runtime = tokio::runtime::Handle::current();
    let handles = rustcode_tui::InteractiveHandles::new();

    match tui {
        TuiCommand::Attach {
            url,
            continue_session,
            session,
            title,
            prompt,
        } => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let mut initial_status = None;

            let resolved_config = match load_effective_config(cli) {
                Ok(config) => config,
                Err(err) => {
                    initial_status = Some(format!("config not loaded: {err}"));
                    ResolvedConfig {
                        allow_network: true,
                        workspace_root: cwd.clone(),
                        model: "remote".to_string(),
                        ..ResolvedConfig::default()
                    }
                }
            };

            if !resolved_config.allow_network {
                initial_status = Some(
                    "network access is disabled; LLM calls will not work (use --allow-network)"
                        .to_string(),
                );
            }

            let defaults = rustcode_tui::InteractiveDefaults {
                workspace_root: resolved_config.workspace_root.clone(),
                model: resolved_config.model.clone(),
            };
            let config = Some(Arc::new(resolved_config));
            let backend: Arc<dyn rustcode_tui::SessionBackend> =
                Arc::new(rustcode_tui::RemoteSessionBackend::new(&url));
            let executor: Option<Arc<dyn rustcode_core::CommandExecutor>> =
                Some(Arc::new(RemoteRunExecutor::new(&url)));

            let mut start = rustcode_tui::InteractiveStart::Sessions;
            if continue_session || session.is_some() || prompt.is_some() {
                let chosen: Result<rustcode_core::SessionInfo> = if continue_session {
                    backend
                        .list_sessions()
                        .and_then(|sessions| {
                            sessions
                                .into_iter()
                                .next()
                                .ok_or_else(|| "no sessions exist to continue".to_string())
                        })
                        .map_err(|err| anyhow::anyhow!("failed to resolve --continue: {err}"))
                } else if let Some(session_id) = session.as_deref() {
                    backend.get_session(session_id).map_err(|err| {
                        anyhow::anyhow!("failed to load session {session_id}: {err}")
                    })
                } else if let Some(prompt_text) = prompt.as_deref() {
                    if prompt_text.trim().is_empty() {
                        Err(anyhow::anyhow!("--prompt must not be empty"))
                    } else {
                        backend
                            .create_session(rustcode_tui::CreateSessionOptions {
                                title: title.clone(),
                                parent_id: None,
                                cwd: cwd.clone(),
                                workspace_root: defaults.workspace_root.clone(),
                                model: defaults.model.clone(),
                            })
                            .map_err(|err| anyhow::anyhow!("failed to create session: {err}"))
                    }
                } else {
                    Err(anyhow::anyhow!("no session selection provided"))
                };

                match chosen {
                    Ok(session) => {
                        let auto_submit = prompt.is_some();
                        start = rustcode_tui::InteractiveStart::Chat {
                            session,
                            prompt: prompt.clone(),
                            auto_submit,
                        };
                    }
                    Err(err) => {
                        if initial_status.is_none() {
                            initial_status = Some(err.to_string());
                        }
                    }
                }
            }

            tokio::task::spawn_blocking(move || {
                let services = rustcode_tui::InteractiveServices {
                    backend,
                    defaults,
                    initial_status,
                    start,
                    runtime,
                    handles,
                    config,
                    executor,
                    submit_mode: rustcode_tui::InteractiveSubmitMode::Run,
                };
                rustcode_tui::run_interactive(services)
            })
            .await
            .context("tui join failed")?
            .context("tui failed")?;
        }
    }
    Ok(())
}

pub async fn handle_tui_default(
    cli: &Cli,
    fork: bool,
    continue_session: bool,
    session: Option<String>,
    prompt: Option<String>,
    title: Option<String>,
) -> Result<()> {
    if !is_interactive_terminal() {
        return Ok(());
    }

    let runtime = tokio::runtime::Handle::current();
    let handles = rustcode_tui::InteractiveHandles::new();
    let store = SessionStore::open_default();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (defaults, mut initial_status, config, executor) = match load_effective_config(cli) {
        Ok(config) => {
            let defaults = rustcode_tui::InteractiveDefaults {
                workspace_root: config.workspace_root.clone(),
                model: config.model.clone(),
            };

            let config = Arc::new(config);
            let (initial_status, executor) = match build_client(&config) {
                Ok(llm_client) => {
                    let recorder: Option<Arc<dyn rustcode_core::TranscriptRecorder>> =
                        Some(Arc::new(FileTranscriptRecorder::new(store.clone())));

                    let mut engine = {
                        let io = Arc::new(LocalIo);
                        Engine::new(
                            llm_client,
                            io.clone(),
                            io,
                            Arc::new(WorkspacePermissionPolicy),
                            PluginRegistry::default(),
                            recorder,
                            Some(handles.approver.clone()),
                        )
                    };

                    if config.allow_network {
                        if let Ok(registry) = build_mcp_registry(&config).await {
                            engine = engine.with_mcp(registry);
                        }
                    }

                    (
                        None,
                        Some(Arc::new(engine) as Arc<dyn rustcode_core::CommandExecutor>),
                    )
                }
                Err(err) => (Some(format!("llm init failed: {err}")), None),
            };

            (defaults, initial_status, Some(config), executor)
        }
        Err(err) => (
            rustcode_tui::InteractiveDefaults {
                workspace_root: cwd.clone(),
                model: "unknown".to_string(),
            },
            Some(format!("config not loaded: {err}")),
            None,
            None,
        ),
    };

    if fork && !continue_session && session.is_none() {
        initial_status = Some("--fork requires --continue or --session <SESSION_ID>".to_string());
    }

    // Default: create a new session and open directly (matches codex/opencode behavior).
    // The session picker is accessible via Ctrl+Q from inside the chat.
    let mut start = if !continue_session && session.is_none() && prompt.is_none() && !fork {
        // Default startup: open a new empty session
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        if let Some(cfg) = config.as_deref() {
            match store.create_session(title.clone(), None, &cwd, &cfg.workspace_root, &cfg.model) {
                Ok(new_session) => rustcode_tui::InteractiveStart::Chat {
                    session: new_session,
                    prompt: None,
                    auto_submit: false,
                },
                Err(_) => rustcode_tui::InteractiveStart::Sessions,
            }
        } else {
            rustcode_tui::InteractiveStart::Sessions
        }
    } else {
        rustcode_tui::InteractiveStart::Sessions
    };

    if !(fork && !continue_session && session.is_none())
        && (continue_session || session.is_some() || prompt.is_some())
    {
        let base: Result<rustcode_core::SessionInfo> = if continue_session {
            match store.list_sessions() {
                Ok(sessions) => sessions
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("no sessions exist to continue")),
                Err(err) => Err(anyhow::anyhow!("failed to list sessions: {err}")),
            }
        } else if let Some(session_id) = session.as_deref() {
            store
                .get_session(session_id)
                .with_context(|| format!("failed to load session {session_id}"))
        } else if let Some(prompt_text) = prompt.as_deref() {
            if prompt_text.trim().is_empty() {
                Err(anyhow::anyhow!("--prompt must not be empty"))
            } else if let Some(config) = config.as_deref() {
                match std::env::current_dir() {
                    Ok(cwd) => store
                        .create_session(
                            title.clone(),
                            None,
                            &cwd,
                            &config.workspace_root,
                            &config.model,
                        )
                        .map_err(|err| anyhow::anyhow!("failed to create session: {err}")),
                    Err(err) => Err(anyhow::anyhow!("failed to resolve cwd: {err}")),
                }
            } else {
                initial_status =
                    Some("config not loaded; cannot create session for --prompt".to_string());
                Err(anyhow::anyhow!("config unavailable"))
            }
        } else {
            Err(anyhow::anyhow!("no session selection provided"))
        };

        match base {
            Ok(base) => {
                let chosen = if fork {
                    match store.fork_session(&base.id, title.clone()) {
                        Ok(forked) => forked,
                        Err(err) => {
                            initial_status =
                                Some(format!("failed to fork session {}: {err}", base.id));
                            base
                        }
                    }
                } else {
                    base
                };

                let auto_submit = prompt.is_some() && config.is_some() && executor.is_some();
                start = rustcode_tui::InteractiveStart::Chat {
                    session: chosen,
                    prompt: prompt.clone(),
                    auto_submit,
                };
            }
            Err(err) => {
                if initial_status.is_none() {
                    initial_status = Some(err.to_string());
                }
            }
        }
    }

    let backend: Arc<dyn rustcode_tui::SessionBackend> =
        Arc::new(rustcode_tui::LocalSessionBackend::new(store.clone()));

    tokio::task::spawn_blocking(move || {
        let services = rustcode_tui::InteractiveServices {
            backend,
            defaults,
            initial_status,
            start,
            runtime,
            handles,
            config,
            executor,
            submit_mode: rustcode_tui::InteractiveSubmitMode::Agent,
        };
        rustcode_tui::run_interactive(services)
    })
    .await
    .context("tui join failed")?
    .context("tui failed")?;
    Ok(())
}
