use super::*;

#[test]
fn invalid_command_is_rejected() {
    let err = Cli::try_parse_from(["rustcode", "not-a-command"]).expect_err("must fail");
    let message = err.to_string();
    assert!(message.contains("unrecognized subcommand"));
    assert!(message.contains("Usage: rustcode"));
}

#[test]
fn invalid_run_flag_is_rejected() {
    let err =
        Cli::try_parse_from(["rustcode", "run", "--not-a-flag", "prompt"]).expect_err("must fail");
    let message = err.to_string();
    assert!(message.contains("unexpected argument '--not-a-flag'"));
    assert!(message.contains("use '-- --not-a-flag'"));
}

#[test]
fn run_accepts_attach_url() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "run",
        "--attach",
        "http://127.0.0.1:4317",
        "hello",
    ])
    .expect("cli should parse");
    match cli.command {
        TopCommand::Run {
            prompt,
            attach,
            continue_session,
            ..
        } => {
            assert_eq!(prompt, "hello");
            assert_eq!(attach.as_deref(), Some("http://127.0.0.1:4317"));
            assert!(!continue_session);
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn exec_accepts_args() {
    let cli = Cli::try_parse_from(["rustcode", "exec", "echo", "hello"]).expect("cli should parse");

    match cli.command {
        TopCommand::Exec { command, args } => {
            assert_eq!(command, "echo");
            assert_eq!(args, vec!["hello"]);
        }
        _ => panic!("expected exec command"),
    }
}

#[test]
fn write_accepts_path_and_contents() {
    let cli = Cli::try_parse_from(["rustcode", "write", "README.md", "hello world"])
        .expect("cli should parse");

    match cli.command {
        TopCommand::Write { path, contents } => {
            assert_eq!(path, "README.md");
            assert_eq!(contents, "hello world");
        }
        _ => panic!("expected write command"),
    }
}

#[test]
fn edit_accepts_path_and_tokens() {
    let cli = Cli::try_parse_from(["rustcode", "edit", "README.md", "old", "new"])
        .expect("cli should parse");

    match cli.command {
        TopCommand::Edit { path, from, to } => {
            assert_eq!(path, "README.md");
            assert_eq!(from, "old");
            assert_eq!(to, "new");
        }
        _ => panic!("expected edit command"),
    }
}

#[test]
fn agent_accepts_prompt() {
    let cli = Cli::try_parse_from(["rustcode", "agent", "hello"]).expect("cli should parse");
    match cli.command {
        TopCommand::Agent { prompt, .. } => {
            assert_eq!(prompt, "hello");
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn agent_accepts_allow_exec_flag() {
    let cli = Cli::try_parse_from(["rustcode", "agent", "--allow-exec", "hello"])
        .expect("cli should parse");
    match cli.command {
        TopCommand::Agent { allow_exec, .. } => {
            assert!(allow_exec);
        }
        _ => panic!("expected agent command"),
    }
}

#[test]
fn global_llm_flags_parse() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "--llm-provider",
        "openrouter",
        "--llm-base-url",
        "https://openrouter.ai/api/v1",
        "--llm-api-key-env",
        "OPENROUTER_API_KEY",
        "run",
        "ping",
    ])
    .expect("cli should parse");

    assert_eq!(cli.llm_provider.as_deref(), Some("openrouter"));
    assert_eq!(
        cli.llm_base_url.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );
    assert_eq!(cli.llm_api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
}

#[test]
fn event_debug_flag_parses_globally() {
    let cli = Cli::try_parse_from(["rustcode", "--event-debug", "run", "ping"])
        .expect("cli should parse");
    assert!(cli.event_debug);
}

#[test]
fn allow_network_flag_parses_globally() {
    let cli = Cli::try_parse_from(["rustcode", "--allow-network", "run", "ping"])
        .expect("cli should parse");
    assert!(cli.allow_network);
    assert!(!cli.deny_network);
}

#[test]
fn allow_network_conflicts_with_deny_network() {
    let err = Cli::try_parse_from([
        "rustcode",
        "--allow-network",
        "--deny-network",
        "run",
        "ping",
    ])
    .expect_err("must reject conflicting flags");
    assert!(err.to_string().contains("cannot be used with"));
}

#[test]
fn auth_set_key_parses_env_source() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "auth",
        "set-key",
        "openrouter",
        "--from-env",
        "OPENROUTER_API_KEY",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Auth {
            command:
                AuthCommand::SetKey {
                    provider,
                    from_env,
                    domain,
                },
        } => {
            assert_eq!(provider, "openrouter");
            assert_eq!(from_env, "OPENROUTER_API_KEY");
            assert!(domain.is_none());
        }
        _ => panic!("expected auth set-key command"),
    }
}

#[test]
fn auth_set_key_accepts_domain() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "auth",
        "set-key",
        "github-copilot-enterprise",
        "--from-env",
        "GITHUB_COPILOT_TOKEN",
        "--domain",
        "github.example.com",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Auth {
            command:
                AuthCommand::SetKey {
                    provider,
                    from_env,
                    domain,
                },
        } => {
            assert_eq!(provider, "github-copilot-enterprise");
            assert_eq!(from_env, "GITHUB_COPILOT_TOKEN");
            assert_eq!(domain.as_deref(), Some("github.example.com"));
        }
        _ => panic!("expected auth set-key command"),
    }
}

#[test]
fn auth_set_oauth_parses_env_sources() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "auth",
        "set-oauth",
        "openai",
        "--access-env",
        "OPENAI_ACCESS_TOKEN",
        "--refresh-env",
        "OPENAI_REFRESH_TOKEN",
        "--expires-unix",
        "1234567890",
        "--account-id",
        "acct_123",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Auth {
            command:
                AuthCommand::SetOauth {
                    provider,
                    access_env,
                    refresh_env,
                    expires_unix,
                    account_id,
                },
        } => {
            assert_eq!(provider, "openai");
            assert_eq!(access_env, "OPENAI_ACCESS_TOKEN");
            assert_eq!(refresh_env.as_deref(), Some("OPENAI_REFRESH_TOKEN"));
            assert_eq!(expires_unix, Some(1234567890));
            assert_eq!(account_id.as_deref(), Some("acct_123"));
        }
        _ => panic!("expected auth set-oauth command"),
    }
}

#[test]
fn auth_list_alias_parses() {
    let cli = Cli::try_parse_from(["rustcode", "auth", "ls"]).expect("cli should parse");
    match cli.command {
        TopCommand::Auth {
            command: AuthCommand::List,
        } => {}
        _ => panic!("expected auth list command"),
    }
}

#[test]
fn auth_methods_parses_with_optional_provider() {
    let with_provider =
        Cli::try_parse_from(["rustcode", "auth", "methods", "openai"]).expect("cli should parse");
    match with_provider.command {
        TopCommand::Auth {
            command: AuthCommand::Methods { provider },
        } => assert_eq!(provider.as_deref(), Some("openai")),
        _ => panic!("expected auth methods command"),
    }

    let without_provider =
        Cli::try_parse_from(["rustcode", "auth", "methods"]).expect("cli should parse");
    match without_provider.command {
        TopCommand::Auth {
            command: AuthCommand::Methods { provider },
        } => assert!(provider.is_none()),
        _ => panic!("expected auth methods command"),
    }
}

#[test]
fn auth_remove_logout_alias_parses() {
    let cli = Cli::try_parse_from(["rustcode", "auth", "logout", "openrouter"])
        .expect("cli should parse");
    match cli.command {
        TopCommand::Auth {
            command: AuthCommand::Remove { provider },
        } => {
            assert_eq!(provider, "openrouter");
        }
        _ => panic!("expected auth remove command"),
    }
}

#[test]
fn models_optional_provider_parses() {
    let cli = Cli::try_parse_from(["rustcode", "models", "openrouter"]).expect("cli should parse");
    match cli.command {
        TopCommand::Models { provider } => {
            assert_eq!(provider.as_deref(), Some("openrouter"));
        }
        _ => panic!("expected models command"),
    }
}

#[test]
fn auth_login_accepts_domain_and_no_wait() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "auth",
        "login",
        "github-copilot-enterprise",
        "--method",
        "oauth_device_code",
        "--domain",
        "company.ghe.com",
        "--oauth-port",
        "1455",
        "--no-wait",
        "--timeout-secs",
        "15",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Auth {
            command:
                AuthCommand::Login {
                    provider,
                    from_env,
                    method,
                    domain,
                    oauth_port,
                    no_wait,
                    timeout_secs,
                },
        } => {
            assert_eq!(provider.as_deref(), Some("github-copilot-enterprise"));
            assert!(from_env.is_none());
            assert_eq!(method.as_deref(), Some("oauth_device_code"));
            assert_eq!(domain.as_deref(), Some("company.ghe.com"));
            assert_eq!(oauth_port, 1455);
            assert!(no_wait);
            assert_eq!(timeout_secs, 15);
        }
        _ => panic!("expected auth login command"),
    }
}

#[test]
fn auth_login_allows_providerless_mode() {
    let cli = Cli::try_parse_from(["rustcode", "auth", "login"]).expect("cli should parse");
    match cli.command {
        TopCommand::Auth {
            command:
                AuthCommand::Login {
                    provider,
                    from_env,
                    method,
                    ..
                },
        } => {
            assert!(provider.is_none());
            assert!(from_env.is_none());
            assert!(method.is_none());
        }
        _ => panic!("expected auth login command"),
    }
}

#[test]
fn mcp_login_parses_from_env_scopes_and_url() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "mcp",
        "login",
        "github",
        "--from-env",
        "RUSTCODE_MCP_TOKEN",
        "--method",
        "oauth_browser",
        "--scopes",
        "read,write",
        "--url",
        "https://example.com/mcp",
        "--oauth-port",
        "19001",
        "--no-wait",
        "--timeout-secs",
        "45",
        "--client-id",
        "client-123",
        "--client-secret-env",
        "MCP_CLIENT_SECRET",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Mcp {
            command:
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
                },
        } => {
            assert_eq!(name, "github");
            assert_eq!(from_env.as_deref(), Some("RUSTCODE_MCP_TOKEN"));
            assert_eq!(method.as_deref(), Some("oauth_browser"));
            assert_eq!(scopes, vec!["read", "write"]);
            assert_eq!(url.as_deref(), Some("https://example.com/mcp"));
            assert_eq!(oauth_port, 19001);
            assert!(no_wait);
            assert_eq!(timeout_secs, 45);
            assert_eq!(client_id.as_deref(), Some("client-123"));
            assert_eq!(client_secret_env.as_deref(), Some("MCP_CLIENT_SECRET"));
        }
        _ => panic!("expected mcp login command"),
    }
}

#[test]
fn mcp_list_alias_parses() {
    let cli = Cli::try_parse_from(["rustcode", "mcp", "ls"]).expect("cli should parse");
    match cli.command {
        TopCommand::Mcp {
            command: McpCommand::List,
        } => {}
        _ => panic!("expected mcp list command"),
    }
}

#[test]
fn mcp_add_parses_with_scope_and_oauth_settings() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "mcp",
        "add",
        "github",
        "--url",
        "https://example.com/mcp",
        "--oauth",
        "on",
        "--client-id",
        "client-123",
        "--client-secret-env",
        "MCP_SECRET",
        "--scope",
        "project",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Mcp {
            command:
                McpCommand::Add {
                    name,
                    url,
                    command,
                    args,
                    env,
                    oauth,
                    client_id,
                    client_secret_env,
                    scope,
                },
        } => {
            assert_eq!(name, "github");
            assert_eq!(url.as_deref(), Some("https://example.com/mcp"));
            assert!(command.is_none());
            assert!(args.is_empty());
            assert!(env.is_empty());
            assert_eq!(oauth.as_deref(), Some("on"));
            assert_eq!(client_id.as_deref(), Some("client-123"));
            assert_eq!(client_secret_env.as_deref(), Some("MCP_SECRET"));
            assert_eq!(scope, "project");
        }
        _ => panic!("expected mcp add command"),
    }
}

#[test]
fn mcp_add_parses_stdio_transport_options() {
    let cli = Cli::try_parse_from([
        "rustcode",
        "mcp",
        "add",
        "local",
        "--command",
        "node",
        "--arg",
        "server.js",
        "--arg=--mcp",
        "--env",
        "MCP_MODE=test",
    ])
    .expect("cli should parse");

    match cli.command {
        TopCommand::Mcp {
            command:
                McpCommand::Add {
                    name,
                    url,
                    command,
                    args,
                    env,
                    ..
                },
        } => {
            assert_eq!(name, "local");
            assert!(url.is_none());
            assert_eq!(command.as_deref(), Some("node"));
            assert_eq!(args, vec!["server.js", "--mcp"]);
            assert_eq!(env, vec!["MCP_MODE=test"]);
        }
        _ => panic!("expected mcp add command"),
    }
}

#[test]
fn mcp_status_parses_with_optional_name() {
    let cli = Cli::try_parse_from(["rustcode", "mcp", "status"]).expect("cli should parse");
    match cli.command {
        TopCommand::Mcp {
            command: McpCommand::Status { name },
        } => {
            assert!(name.is_none());
        }
        _ => panic!("expected mcp status command"),
    }

    let cli =
        Cli::try_parse_from(["rustcode", "mcp", "status", "github"]).expect("cli should parse");
    match cli.command {
        TopCommand::Mcp {
            command: McpCommand::Status { name },
        } => {
            assert_eq!(name.as_deref(), Some("github"));
        }
        _ => panic!("expected mcp status command"),
    }
}
