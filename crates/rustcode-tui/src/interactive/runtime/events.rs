use rustcode_core::event::EventScope;

use super::super::{
    push_toast, ActivityItem, AppState, ApprovalMode, ChatFocus, Duration, EventPayload, GitStat,
    InteractiveMsg, MessageRole, Modal, PendingApproval, ProviderManagerStep, Screen, ToastVariant,
};
use super::git::refresh_git_stat_async;

/// Drain **all** queued messages without blocking.
///
/// Called on draw frames to catch up with any messages that arrived between
/// the last `select!` wakeup and the current render.  In the async
/// architecture, render starvation is prevented by the `select!` loop
/// itself, so there is no need to cap the drain count.
pub(super) fn drain_messages(state: &mut AppState) {
    loop {
        let msg = match state.rx.try_recv() {
            Ok(msg) => msg,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return,
        };
        process_message(state, msg);
    }
}

/// Process a single [`InteractiveMsg`].
///
/// Called by the async `select!` loop for individual messages, and by
/// [`drain_messages`] for batch catch-up on draw frames.
pub(super) fn process_message(state: &mut AppState, msg: InteractiveMsg) {
    let mut screen = std::mem::replace(&mut state.screen, Screen::Sessions);
    match msg {
        InteractiveMsg::EngineEvent(event) => {
            if let Screen::Chat(chat) = &mut screen {
                let item = match &event.payload {
                    EventPayload::CommandAccepted { name } => {
                        Some(ActivityItem::CommandAccepted { name: name.clone() })
                    }
                    EventPayload::ToolCall {
                        id,
                        name,
                        arguments,
                    } => Some(ActivityItem::ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }),
                    EventPayload::ToolResult {
                        id,
                        name,
                        ok,
                        output,
                    } => Some(ActivityItem::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        ok: *ok,
                        output: output.clone(),
                    }),
                    EventPayload::OutputChunk { text } => {
                        if event.scope == EventScope::Command {
                            chat.live_assistant.push_str(text);
                            if chat.live_assistant.len() > 64 * 1024 {
                                let keep = 48 * 1024;
                                let mut start = chat.live_assistant.len().saturating_sub(keep);
                                while start < chat.live_assistant.len()
                                    && !chat.live_assistant.is_char_boundary(start)
                                {
                                    start += 1;
                                }
                                chat.live_assistant = chat.live_assistant[start..].to_string();
                            }
                        }
                        Some(ActivityItem::OutputChunk { text: text.clone() })
                    }
                    EventPayload::ReasoningChunk { text } => {
                        chat.live_reasoning.push_str(text);
                        if chat.live_reasoning.len() > 64 * 1024 {
                            let keep = 48 * 1024;
                            let mut start = chat.live_reasoning.len().saturating_sub(keep);
                            while start < chat.live_reasoning.len()
                                && !chat.live_reasoning.is_char_boundary(start)
                            {
                                start += 1;
                            }
                            chat.live_reasoning = chat.live_reasoning[start..].to_string();
                        }
                        Some(ActivityItem::ReasoningChunk { text: text.clone() })
                    }
                    EventPayload::Warning { message } => {
                        push_toast(
                            state,
                            ToastVariant::Warning,
                            message.clone(),
                            Duration::from_secs(4),
                        );
                        Some(ActivityItem::Warning {
                            message: message.clone(),
                        })
                    }
                    EventPayload::Failure { message } => {
                        // Show a clean, actionable message in transcript/toast.
                        // Raw provider errors are often too noisy for UI.
                        let clean = clean_failure_message(message);
                        state.status = Some(clean.clone());
                        push_toast(
                            state,
                            ToastVariant::Error,
                            clean.clone(),
                            Duration::from_secs(6),
                        );
                        Some(ActivityItem::Failure {
                            message: clean,
                            raw_detail: Some(message.clone()),
                        })
                    }
                    EventPayload::RetryAttempt {
                        attempt,
                        max_retries,
                        delay_secs,
                        reason,
                    } => {
                        let toast_msg = format!(
                            "Retrying ({attempt}/{max_retries}) in {delay_secs}s: {reason}"
                        );
                        push_toast(
                            state,
                            ToastVariant::Warning,
                            toast_msg,
                            Duration::from_secs((*delay_secs).max(3)),
                        );
                        Some(ActivityItem::RetryAttempt {
                            attempt: *attempt,
                            max_retries: *max_retries,
                            delay_secs: *delay_secs,
                            reason: reason.clone(),
                        })
                    }
                    EventPayload::Completed => {
                        chat.running = None;
                        chat.composer_cleared_by_ctrl_c = false;
                        // Clear live plan/todo widgets so they don't
                        // persist into the next conversation turn.
                        chat.plan_title = None;
                        chat.plan_steps.clear();
                        chat.todos.clear();
                        push_toast(
                            state,
                            ToastVariant::Success,
                            "completed",
                            Duration::from_secs(2),
                        );
                        Some(ActivityItem::Completed)
                    }
                    EventPayload::UsageUpdate {
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        context_limit,
                        ..
                    } => {
                        chat.total_input_tokens += input_tokens;
                        chat.total_output_tokens += output_tokens;
                        chat.last_total_tokens = *total_tokens;
                        chat.context_limit = *context_limit;
                        // Estimate cost: ~$3/Mtok input, ~$15/Mtok output (avg across providers)
                        let step_cost = (*input_tokens as f64 * 3.0 + *output_tokens as f64 * 15.0)
                            / 1_000_000.0;
                        chat.cost_usd += step_cost;
                        None
                    }
                    EventPayload::PlanUpdate { title, steps } => {
                        chat.plan_title = Some(title.clone());
                        chat.plan_steps = steps
                            .iter()
                            .map(|s| (s.description.clone(), s.status.clone()))
                            .collect();
                        None
                    }
                    EventPayload::TodoUpdate { todos } => {
                        chat.todos = todos
                            .iter()
                            .map(|t| (t.content.clone(), t.status.clone(), t.priority.clone()))
                            .collect();
                        None
                    }
                    EventPayload::ServeRequest { .. } => None,
                };

                let was_at_tail = chat.activity_selected >= chat.activity.len().saturating_sub(1);
                let auto_follow_tail = chat.focus != ChatFocus::Activity || was_at_tail;

                if let Some(item) = item {
                    chat.activity.push_back(item);
                }
                while chat.activity.len() > 200 {
                    chat.activity.pop_front();
                    if chat.activity_selected > 0 {
                        chat.activity_selected -= 1;
                    }
                }
                if (auto_follow_tail && !chat.activity.is_empty())
                    || chat.activity_selected >= chat.activity.len()
                {
                    chat.activity_selected = chat.activity.len().saturating_sub(1);
                }
            }
        }
        InteractiveMsg::ApprovalRequest { request, reply } => {
            match state.approval_mode {
                ApprovalMode::Yolo => {
                    // Auto-approve everything — no committed_approvals push;
                    // diffs are visible via the live_activity feed.
                    let _ = reply.send(crate::ApprovalResponse::AllowOnce);
                }
                ApprovalMode::Plan => {
                    // Auto-deny everything (read-only mode).
                    let _ = reply.send(crate::ApprovalResponse::Deny);
                    push_toast(
                        state,
                        ToastVariant::Info,
                        format!("Plan mode — denied [{}]", request.tool),
                        Duration::from_secs(2),
                    );
                }
                ApprovalMode::AcceptEdits => {
                    let is_cmd = super::super::approval_is_command_permission(&request.permission);
                    if is_cmd {
                        // Command → prompt user.
                        state.pending_approval = Some(PendingApproval { request, reply });
                        state.approval_selection = 0;
                    } else {
                        // File/edit tool → auto-approve; diffs visible via live_activity.
                        let _ = reply.send(crate::ApprovalResponse::AllowOnce);
                    }
                }
                ApprovalMode::Normal => {
                    state.pending_approval = Some(PendingApproval { request, reply });
                    state.approval_selection = 0;
                }
            }
        }
        InteractiveMsg::RunEnded { ok, message } => {
            // Phase 1: immediate state updates (non-blocking).
            if let Screen::Chat(chat) = &mut screen {
                if let Some(started) = chat.run_started_at.take() {
                    chat.last_run_elapsed = Some(started.elapsed());
                }
                chat.running = None;
                chat.composer_cleared_by_ctrl_c = false;
                if ok {
                    state.status = None;
                } else {
                    let clean = message.as_deref().map(clean_failure_message);
                    state.status = clean.clone();
                    if let Some(msg) = clean {
                        let has_failure = chat
                            .activity
                            .iter()
                            .any(|item| matches!(item, ActivityItem::Failure { .. }));
                        if !has_failure {
                            chat.activity.push_back(ActivityItem::Failure {
                                message: msg.clone(),
                                raw_detail: message.clone(),
                            });
                        }
                        push_toast(state, ToastVariant::Error, msg, Duration::from_secs(6));
                    }
                }

                // Phase 2: offload blocking I/O (disk reads, git subprocess) to
                // a background thread so the event loop stays responsive.
                let session_id = chat.session.id.clone();
                let session_title = chat.session.title.clone();
                let session_model = chat.session.model.clone();
                let tokens_in = chat.total_input_tokens;
                let tokens_out = chat.total_output_tokens;
                let cost = chat.cost_usd;
                let live_reasoning = chat.live_reasoning.clone();
                let backend = state.backend.clone();
                let tx = state.tx.clone();
                let workspace = state.defaults.workspace_root.clone();
                tokio::task::spawn_blocking(move || {
                    // Persist usage (fire-and-forget).
                    if ok {
                        let _ =
                            backend.update_session_usage(&session_id, tokens_in, tokens_out, cost);
                    }
                    // Reload transcript from disk.
                    let messages = backend.load_messages(&session_id);

                    // Auto-rename untitled session from first user message.
                    let session_update = if ok && session_title.is_none() {
                        auto_rename_session(
                            &*backend,
                            &session_id,
                            &session_model,
                            &live_reasoning,
                            messages.as_deref().ok(),
                        )
                    } else {
                        None
                    };

                    let _ = tx.send(InteractiveMsg::RunEndedIo {
                        ok,
                        messages,
                        session_update,
                    });
                });

                // Also refresh git stats in background.
                if ok {
                    refresh_git_stat_async(workspace, state.tx.clone());
                }
            }
        }
        InteractiveMsg::RunEndedIo {
            ok,
            messages,
            session_update,
        } => {
            if let Screen::Chat(chat) = &mut screen {
                match messages {
                    Ok(messages) => {
                        let was_at_bottom = chat.scroll == 0;
                        chat.messages = messages;
                        if was_at_bottom {
                            chat.scroll = 0;
                        }
                        chat.committed_approvals.clear();
                        chat.pending_prompt = None;
                        if ok {
                            if !chat.live_reasoning.trim().is_empty() {
                                let has_assistant_reasoning = chat
                                    .messages
                                    .iter()
                                    .rev()
                                    .find(|msg| msg.role == MessageRole::Assistant)
                                    .and_then(|msg| msg.reasoning.as_deref())
                                    .is_some_and(|text| !text.trim().is_empty());
                                if !has_assistant_reasoning {
                                    if let Some(last_assistant) = chat
                                        .messages
                                        .iter_mut()
                                        .rev()
                                        .find(|msg| msg.role == MessageRole::Assistant)
                                    {
                                        last_assistant.reasoning =
                                            Some(chat.live_reasoning.clone());
                                    }
                                }
                            }
                            chat.live_assistant.clear();
                            chat.live_reasoning.clear();
                        } else {
                            chat.live_assistant.clear();
                            chat.live_reasoning.clear();
                        }
                        if let Some(mut info) = session_update {
                            info.model = chat.session.model.clone();
                            chat.session = info;
                        }
                    }
                    Err(err) => {
                        state.status = Some(format!("failed to load transcript: {err}"));
                        push_toast(
                            state,
                            ToastVariant::Error,
                            format!("failed to load transcript: {err}"),
                            Duration::from_secs(4),
                        );
                    }
                }
            }
        }
        InteractiveMsg::GitStatUpdate {
            files,
            insertions,
            deletions,
        } => {
            state.git_stat = Some(GitStat {
                files,
                insertions,
                deletions,
            });
        }
    }

    state.screen = screen;
}

/// Poll the OAuth background-task channels and update the provider manager modal.
///
/// Called every loop iteration (50 ms). Non-blocking — uses `try_recv`.
pub(super) fn poll_provider_oauth(state: &mut AppState) {
    // Poll the "device code started" channel.
    if let Some(rx) = &mut state.provider_oauth_start_rx {
        match rx.try_recv() {
            Ok(Ok(started)) => {
                if let Some(Modal::ProviderManager { step }) = &mut state.modal {
                    if let ProviderManagerStep::OAuthStarting { display_name, .. } = step {
                        let display_name = display_name.clone();
                        *step = ProviderManagerStep::OAuthPending {
                            provider_id: started.provider_id,
                            display_name,
                            verification_uri: started.verification_uri,
                            user_code: started.user_code,
                        };
                    }
                }
                state.provider_oauth_start_rx = None;
            }
            Ok(Err(err)) => {
                push_toast(
                    state,
                    ToastVariant::Error,
                    format!("OAuth failed to start: {err}"),
                    Duration::from_secs(6),
                );
                state.modal = None;
                state.provider_oauth_start_rx = None;
                state.provider_oauth_done_rx = None;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                state.provider_oauth_start_rx = None;
            }
        }
    }

    // Poll the "credential ready" channel.
    if let Some(rx) = &mut state.provider_oauth_done_rx {
        match rx.try_recv() {
            Ok(Ok(done)) => {
                let store = rustcode_auth::AuthStore::open_default();
                match store.set_oauth(
                    &done.provider_id,
                    &done.access_token,
                    done.refresh_token.as_deref(),
                    done.expires_at_unix,
                    done.account_id.as_deref(),
                ) {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Success,
                        format!("{} connected!", done.provider_id),
                        Duration::from_secs(4),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to save credential: {err}"),
                        Duration::from_secs(6),
                    ),
                }
                state.modal = None;
                state.provider_oauth_done_rx = None;
            }
            Ok(Err(err)) => {
                push_toast(
                    state,
                    ToastVariant::Error,
                    format!("OAuth authorization failed: {err}"),
                    Duration::from_secs(6),
                );
                state.modal = None;
                state.provider_oauth_done_rx = None;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                state.provider_oauth_done_rx = None;
            }
        }
    }
}

/// Convert a raw executor/LLM error string into a short, actionable message
/// suitable for the status footer and toast notification.
///
/// The full technical detail is preserved in `ActivityItem::Failure` for
/// debugging via Ctrl+E.  The pattern matching targets the `{kind:?}` Debug
/// representations embedded by `LlmError::Classified`'s Display impl.
fn clean_failure_message(msg: &str) -> String {
    let lower = msg.to_lowercase();

    // Auth failures — API key missing or rejected
    if lower.contains("authfailed")
        || lower.contains("authentication failed")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("unauthorized")
        || lower.contains("api key")
        || (lower.contains("401") && lower.contains("provider"))
    {
        return "Authentication failed — connect your provider in /providers (Ctrl+A)".to_string();
    }

    // Rate limit / quota exhausted
    if lower.contains("ratelimit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
        || lower.contains("429")
    {
        return "Rate limit or usage quota reached — please wait before retrying".to_string();
    }

    // Context window overflow
    if lower.contains("contextoverflow")
        || lower.contains("context limit")
        || lower.contains("context_length_exceeded")
        || lower.contains("context window")
        || lower.contains("maximum context")
    {
        return "Context limit exceeded — use /compact or send a shorter message".to_string();
    }

    // Provider service temporarily unavailable
    if lower.contains("serviceunavailable")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("overloaded")
        || lower.contains("internal server error")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
    {
        return "Provider temporarily unavailable — please try again".to_string();
    }

    // Invalid request
    if lower.contains("invalidrequest") {
        return "Request rejected by provider — see activity for details".to_string();
    }

    // Config / no executor — no provider connected
    if lower.contains("configuration error")
        || lower.contains("not set")
        || lower.contains("api key env")
        || lower.contains("executor not available")
        || lower.contains("llm init failed")
    {
        return "No provider connected — use /providers (Ctrl+A) to connect one".to_string();
    }

    // Specific timeout patterns (before generic network catch-all)
    if lower.contains("timed out waiting for provider response chunk") {
        return "Stream timed out — model may be slow to respond, try again".to_string();
    }
    if lower.contains("timed out waiting for provider response headers") {
        return "Connection timed out — provider did not respond, retries exhausted".to_string();
    }

    // Connection errors
    if lower.contains("connection reset") || lower.contains("connection refused") {
        return "Connection lost — check your network and try again".to_string();
    }

    // Generic network / transport errors
    if lower.contains("network error")
        || lower.contains("connection")
        || lower.contains("timed out")
        || lower.contains("transport")
    {
        return "Network error — check your connection and try again".to_string();
    }

    // Generic: strip "provider error (X): " prefix to expose the clean body
    if let Some(idx) = msg.find("): ") {
        let rest = msg[idx + 3..].trim();
        if !rest.is_empty() && rest.len() < 120 {
            return format!("Request failed: {rest}");
        }
    }

    // Last resort: return as-is but capped at 120 chars
    let capped: String = msg.chars().take(120).collect();
    if capped.len() < msg.len() {
        format!("{capped}…")
    } else {
        capped
    }
}

/// Auto-rename an untitled session from the first user message.
///
/// Runs on a blocking thread — performs disk I/O via `update_session_title`.
fn auto_rename_session(
    backend: &dyn crate::SessionBackend,
    session_id: &str,
    session_model: &str,
    _live_reasoning: &str,
    messages: Option<&[rustcode_core::StoredMessage]>,
) -> Option<rustcode_core::SessionInfo> {
    let messages = messages?;
    let first_user = messages.iter().find(|m| m.role == MessageRole::User)?;
    let text = first_user.content.as_str()?;
    let clean = text.trim().replace('\n', " ");
    if clean.is_empty() {
        return None;
    }
    let title = if clean.chars().count() > 50 {
        let short: String = clean.chars().take(50).collect();
        if let Some(pos) = short.rfind(' ') {
            format!("{}…", &short[..pos])
        } else {
            format!("{short}…")
        }
    } else {
        clean
    };
    let mut info = backend.update_session_title(session_id, Some(title)).ok()?;
    // Renaming must never alter the active model label.
    info.model = session_model.to_string();
    Some(info)
}
