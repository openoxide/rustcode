use anyhow::Result;

use crate::cli::TopCommand;

pub(crate) fn build_engine_command(
    command: TopCommand,
    run_history: &[rustcode_core::session::StoredMessage],
    agent_history: Vec<rustcode_core::session::StoredMessage>,
) -> Result<rustcode_core::command::Command> {
    let command = match command {
        TopCommand::Run {
            prompt,
            attach: None,
            ..
        } => rustcode_core::command::Command::Run {
            prompt: render_run_prompt(prompt, run_history),
        },
        TopCommand::Run {
            attach: Some(_), ..
        } => {
            anyhow::bail!("internal error: attach runs must be handled before engine dispatch")
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
        } => rustcode_core::command::Command::Agent {
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
                mode_hint: None,
            },
            history: agent_history,
        },
        TopCommand::Exec { command, args } => {
            rustcode_core::command::Command::Exec { command, args }
        }
        TopCommand::List { path } => rustcode_core::command::Command::List { path },
        TopCommand::Read { path } => rustcode_core::command::Command::Read { path },
        TopCommand::Write { path, contents } => rustcode_core::command::Command::Write { path, contents },
        TopCommand::Edit { path, from, to } => rustcode_core::command::Command::Edit { path, from, to },
        TopCommand::Tui(_) => rustcode_core::command::Command::Tui,
        TopCommand::Serve { listen } => rustcode_core::command::Command::Serve { listen },
        TopCommand::Version => rustcode_core::command::Command::Version,
        TopCommand::Models { .. } => {
            anyhow::bail!("internal error: models command must be handled before engine dispatch")
        }
        TopCommand::Auth { .. } => {
            anyhow::bail!("internal error: auth command must be handled before engine dispatch")
        }
        TopCommand::Mcp { .. } => {
            anyhow::bail!("internal error: mcp command must be handled before engine dispatch")
        }
        TopCommand::Session { .. } => {
            anyhow::bail!("internal error: session command must be handled before engine dispatch")
        }
        TopCommand::Export { .. } => {
            anyhow::bail!("internal error: export command must be handled before engine dispatch")
        }
        TopCommand::Import { .. } => {
            anyhow::bail!("internal error: import command must be handled before engine dispatch")
        }
        TopCommand::GitHub { .. } => {
            anyhow::bail!("internal error: github command must be handled before engine dispatch")
        }
        TopCommand::Pr { .. } => {
            anyhow::bail!("internal error: pr command must be handled before engine dispatch")
        }
        TopCommand::Worktree { .. } => {
            anyhow::bail!("internal error: worktree command must be handled before engine dispatch")
        }
    };

    Ok(command)
}

fn render_run_prompt(
    prompt: String,
    run_history: &[rustcode_core::session::StoredMessage],
) -> String {
    if run_history.is_empty() {
        return prompt;
    }

    let mut rendered = String::new();
    for msg in run_history {
        let Some(text) = msg.content.as_str() else {
            continue;
        };
        let role = match msg.role {
            rustcode_core::session::MessageRole::System => "System",
            rustcode_core::session::MessageRole::User => "User",
            rustcode_core::session::MessageRole::Assistant => "Assistant",
            rustcode_core::session::MessageRole::Tool => "Tool",
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
}
