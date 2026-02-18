use clap::{Args, Parser, Subcommand};

use rustcode_core::command::AgentOptions;

#[derive(Debug, Parser)]
#[command(
    name = "rustcode",
    about = "Production-grade CLI/TUI systems tool",
    version
)]
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

    #[arg(
        long = "allow-network",
        global = true,
        default_value_t = false,
        conflicts_with = "deny_network"
    )]
    pub allow_network: bool,

    #[arg(long = "deny-network", global = true, default_value_t = false)]
    pub deny_network: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TopCommand {
    Run {
        prompt: String,

        #[arg(long, value_name = "URL", conflicts_with = "continue_session")]
        attach: Option<String>,

        #[arg(long = "continue", default_value_t = false, conflicts_with = "session")]
        continue_session: bool,

        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        #[arg(long, default_value_t = false)]
        fork: bool,

        #[arg(long)]
        title: Option<String>,
    },
    Agent {
        prompt: String,
        #[arg(long = "max-steps", default_value_t = AgentOptions::default().max_steps)]
        max_steps: usize,
        #[arg(
            long = "max-tool-calls",
            default_value_t = AgentOptions::default().max_tool_calls_per_step
        )]
        max_tool_calls_per_step: usize,
        #[arg(long = "allow-write", default_value_t = false)]
        allow_write: bool,
        #[arg(long = "allow-edit", default_value_t = false)]
        allow_edit: bool,
        #[arg(long = "allow-exec", default_value_t = false)]
        allow_exec: bool,
        #[arg(
            long = "max-read-bytes",
            default_value_t = AgentOptions::default().max_read_bytes
        )]
        max_read_bytes: usize,
        #[arg(
            long = "max-list-entries",
            default_value_t = AgentOptions::default().max_list_entries
        )]
        max_list_entries: usize,
        #[arg(
            long = "max-tool-result-bytes",
            default_value_t = AgentOptions::default().max_tool_result_bytes
        )]
        max_tool_result_bytes: usize,
        #[arg(
            long = "max-write-bytes",
            default_value_t = AgentOptions::default().max_write_bytes
        )]
        max_write_bytes: usize,

        #[arg(long = "continue", default_value_t = false, conflicts_with = "session")]
        continue_session: bool,

        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        #[arg(long, default_value_t = false)]
        fork: bool,

        #[arg(long)]
        title: Option<String>,
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
    Tui(TuiArgs),
    Serve {
        #[arg(long, default_value = "127.0.0.1:4317")]
        listen: String,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Export {
        session_id: Option<String>,
    },
    Import {
        file: String,
    },
    #[command(name = "github")]
    GitHub {
        #[command(subcommand)]
        command: GitHubCommand,
    },
    #[command(name = "pr")]
    Pr {
        #[command(subcommand)]
        command: PrCommand,
    },
    Version,
}

#[derive(Debug, Clone, Subcommand)]
pub enum GitHubCommand {
    Status,
    Repo,
}

#[derive(Debug, Clone, Subcommand)]
pub enum PrCommand {
    Checkout {
        number: u32,

        #[arg(long)]
        branch: Option<String>,

        #[arg(long, default_value_t = false)]
        force: bool,

        #[arg(long, default_value_t = false)]
        tui: bool,

        #[arg(
            long = "tui-continue",
            default_value_t = false,
            requires = "tui",
            conflicts_with = "tui_session"
        )]
        tui_continue_session: bool,

        #[arg(long = "tui-session", value_name = "SESSION_ID", requires = "tui")]
        tui_session: Option<String>,

        #[arg(long = "tui-title", requires = "tui")]
        tui_title: Option<String>,

        #[arg(long = "tui-prompt", requires = "tui")]
        tui_prompt: Option<String>,
    },
    Create {
        #[arg(long)]
        title: Option<String>,

        #[arg(long)]
        body: Option<String>,

        #[arg(long)]
        base: Option<String>,

        #[arg(long)]
        head: Option<String>,

        #[arg(long, default_value_t = false)]
        draft: bool,

        #[arg(long, default_value_t = false, conflicts_with_all = ["title", "body"])]
        fill: bool,
    },
}

#[derive(Debug, Clone, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct TuiArgs {
    #[command(subcommand)]
    pub command: Option<TuiCommand>,

    // When no subcommand is provided, these flags control local TUI startup.
    #[arg(long = "continue", default_value_t = false, conflicts_with = "session")]
    pub continue_session: bool,

    #[arg(long, value_name = "SESSION_ID")]
    pub session: Option<String>,

    #[arg(long, default_value_t = false)]
    pub fork: bool,

    #[arg(long)]
    pub title: Option<String>,

    #[arg(long)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TuiCommand {
    Attach {
        url: String,

        #[arg(long = "continue", default_value_t = false, conflicts_with = "session")]
        continue_session: bool,

        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,

        #[arg(long)]
        title: Option<String>,

        #[arg(long)]
        prompt: Option<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    #[command(alias = "ls")]
    List,
    New {
        #[arg(long)]
        title: Option<String>,
    },
    Fork {
        session_id: String,
        #[arg(long)]
        title: Option<String>,
    },
    Show {
        session_id: String,
    },
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
        #[arg(long)]
        domain: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpLoginMethod {
    ApiKey,
    OAuthBrowser,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpCommand {
    #[command(alias = "ls")]
    List,
    Status {
        name: Option<String>,
    },
    Get {
        name: String,
    },
    Add {
        name: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long = "env")]
        env: Vec<String>,
        #[arg(long = "oauth", value_parser = ["on", "off"])]
        oauth: Option<String>,
        #[arg(long = "client-id")]
        client_id: Option<String>,
        #[arg(long = "client-secret-env")]
        client_secret_env: Option<String>,
        #[arg(long = "scope", value_parser = ["user", "project"], default_value = "user")]
        scope: String,
    },
    Remove {
        name: String,
        #[arg(long = "scope", value_parser = ["user", "project"], default_value = "user")]
        scope: String,
    },
    Login {
        name: String,
        #[arg(long = "from-env")]
        from_env: Option<String>,
        #[arg(long = "method", value_parser = ["token_import", "oauth_browser"])]
        method: Option<String>,
        #[arg(long, value_delimiter = ',', value_name = "SCOPE,SCOPE")]
        scopes: Vec<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long = "oauth-port", default_value_t = 8788)]
        oauth_port: u16,
        #[arg(long, default_value_t = false)]
        no_wait: bool,
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
        #[arg(long = "client-id")]
        client_id: Option<String>,
        #[arg(long = "client-secret-env")]
        client_secret_env: Option<String>,
    },
    Logout {
        name: String,
    },
}

#[cfg(test)]
mod tests;
