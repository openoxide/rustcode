use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossterm::event::{
    self, Event as CEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use tokio_util::sync::CancellationToken;

use rustcode_core::command::AgentOptions;
use rustcode_core::event::EventPayload;
use rustcode_core::ports::EventPublisher;
use rustcode_core::{Command, CommandContext, ResolvedConfig, SessionInfo};
use rustcode_core::{MessageRole, SessionMeta, StoredMessage, ToolApprovalRequest};

use crate::{
    ApprovalResponse, CreateSessionOptions, InteractiveDefaults, InteractiveMsg,
    InteractiveServices, InteractiveStart, InteractiveSubmitMode, TuiError, TuiPublisher,
};

mod commands;
mod composer;
mod file_search;
mod input;
mod markdown;
mod palette;
mod provider_manager;
mod render_activity;
mod render_approval;
mod render_main;
mod render_modals;
mod render_provider;
mod runtime;
mod state;
mod syntax_highlight;
mod transcript;
mod types;

use commands::{
    execute_command, filter_models, handle_slash_command, open_session_by_id,
    refresh_chat_messages, SLASH_COMMANDS,
};
use composer::{
    composer_backspace, composer_clear, composer_cursor_visual, composer_delete,
    composer_insert_str, composer_kill_line_backward, composer_kill_line_forward,
    composer_move_down, composer_move_end, composer_move_home, composer_move_left,
    composer_move_right, composer_move_up, composer_word_left, composer_word_right, history_next,
    history_prev,
};
use file_search::{filter_files, scan_workspace_files};
use input::handle_key;
use palette::{compute_palette_view, maybe_execute_palette_query, open_command_palette};
use provider_manager::{
    build_provider_entries, filter_provider_entries, provider_connect_methods,
    provider_display_name, provider_env_hint,
};
use render_activity::{render_activity, render_activity_details_modal};
use render_approval::{render_approval_inline, render_approval_selector};
use render_main::render;
use render_modals::{centered_rect, render_help_modal, render_modal};
use render_provider::render_provider_manager_modal;
use runtime::submit_prompt;
use state::{
    compute_sessions_view, drain_toasts, format_age, push_toast, sort_sessions,
    transcript_area_height,
};
use transcript::{
    apply_find_highlight, build_transcript_lines, compute_find_matches, find_next, find_prev,
    set_find,
};
use types::{
    build_prompt_history, ActivityItem, AppState, ChatFocus, ChatNav, ChatState, CommandId,
    CommandItem, ConnectMethod, FindState, GitStat, Modal, PendingApproval, ProviderEntry,
    ProviderManagerStep, ProviderOAuthDone, ProviderOAuthStarted, RunningCommand, Screen, Toast,
    ToastVariant,
};

pub fn run_interactive(services: InteractiveServices) -> Result<(), TuiError> {
    runtime::run_interactive(services)
}

#[cfg(test)]
mod tests;
