use clap::{Parser, Subcommand};

use rustcode_core::command::Command;

#[derive(Debug, Parser)]
#[command(name = "rustcode", about = "Production-grade CLI/TUI systems tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: TopCommand,

    #[arg(long, global = true)]
    pub json: bool,

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

#[derive(Debug, Subcommand)]
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
    Version,
}

pub fn map_command(command: TopCommand) -> Command {
    match command {
        TopCommand::Run { prompt } => Command::Run { prompt },
        TopCommand::Exec { command, args } => Command::Exec { command, args },
        TopCommand::List { path } => Command::List { path },
        TopCommand::Read { path } => Command::Read { path },
        TopCommand::Write { path, contents } => Command::Write { path, contents },
        TopCommand::Edit { path, from, to } => Command::Edit { path, from, to },
        TopCommand::Tui => Command::Tui,
        TopCommand::Serve { listen } => Command::Serve { listen },
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
}
