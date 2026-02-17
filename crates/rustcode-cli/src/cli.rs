use clap::{Parser, Subcommand};

use rustcode_core::command::Command;

#[derive(Debug, Parser)]
#[command(name = "rustcode", about = "Production-grade CLI/TUI systems tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: TopCommand,

    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long = "event-debug", global = true, default_value_t = false)]
    pub event_debug: bool,

    #[arg(long, global = true)]
    pub profile: Option<String>,

    #[arg(long, global = true)]
    pub model: Option<String>,

    #[arg(long = "llm-provider", global = true)]
    pub llm_provider: Option<String>,

    #[arg(long = "llm-base-url", global = true)]
    pub llm_base_url: Option<String>,

    #[arg(long = "llm-api-key-env", global = true)]
    pub llm_api_key_env: Option<String>,

    #[arg(long = "trust-project-config", global = true)]
    pub trust_project_config: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TopCommand {
    Run {
        prompt: String,
    },
    Exec {
        command: String,
        args: Vec<String>,
    },
    List {
        path: Option<String>,
    },
    Models {
        provider: Option<String>,
    },
    Read {
        path: String,
    },
    Write {
        path: String,
        contents: String,
    },
    Edit {
        path: String,
        from: String,
        to: String,
    },
    Tui,
    Serve {
        #[arg(long, default_value = "127.0.0.1:4317")]
        listen: String,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Version,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthCommand {
    #[command(alias = "ls")]
    List,
    Methods {
        provider: Option<String>,
    },
    Status {
        provider: String,
    },
    SetKey {
        provider: String,
        #[arg(long = "from-env")]
        from_env: String,
    },
    SetOauth {
        provider: String,
        #[arg(long = "access-env")]
        access_env: String,
        #[arg(long = "refresh-env")]
        refresh_env: Option<String>,
        #[arg(long = "expires-unix")]
        expires_unix: Option<i64>,
        #[arg(long = "account-id")]
        account_id: Option<String>,
    },
    #[command(alias = "logout")]
    Remove {
        provider: String,
    },
    Login {
        provider: Option<String>,
        #[arg(long = "from-env")]
        from_env: Option<String>,
        #[arg(long = "method", value_parser = ["api_key", "oauth_device_code", "oauth_browser"])]
        method: Option<String>,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long = "oauth-port", default_value_t = 8080)]
        oauth_port: u16,
        #[arg(long, default_value_t = false)]
        no_wait: bool,
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
}

pub fn map_command(command: TopCommand) -> Command {
    match command {
        TopCommand::Run { prompt } => Command::Run { prompt },
        TopCommand::Exec { command, args } => Command::Exec { command, args },
        TopCommand::List { path } => Command::List { path },
        TopCommand::Models { .. } => {
            panic!("models command is handled in cli main before engine dispatch")
        }
        TopCommand::Read { path } => Command::Read { path },
        TopCommand::Write { path, contents } => Command::Write { path, contents },
        TopCommand::Edit { path, from, to } => Command::Edit { path, from, to },
        TopCommand::Tui => Command::Tui,
        TopCommand::Serve { listen } => Command::Serve { listen },
        TopCommand::Auth { .. } => {
            panic!("auth command is handled in cli main before engine dispatch")
        }
        TopCommand::Version => Command::Version,
    }
}

#[cfg(test)]
mod tests {
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
        let err = Cli::try_parse_from(["rustcode", "run", "--not-a-flag", "prompt"])
            .expect_err("must fail");
        let message = err.to_string();
        assert!(message.contains("unexpected argument '--not-a-flag'"));
        assert!(message.contains("use '-- --not-a-flag'"));
    }

    #[test]
    fn exec_accepts_args() {
        let cli =
            Cli::try_parse_from(["rustcode", "exec", "echo", "hello"]).expect("cli should parse");

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
                command: AuthCommand::SetKey { provider, from_env },
            } => {
                assert_eq!(provider, "openrouter");
                assert_eq!(from_env, "OPENROUTER_API_KEY");
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
        let with_provider = Cli::try_parse_from(["rustcode", "auth", "methods", "openai"])
            .expect("cli should parse");
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
        let cli =
            Cli::try_parse_from(["rustcode", "models", "openrouter"]).expect("cli should parse");
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
}
