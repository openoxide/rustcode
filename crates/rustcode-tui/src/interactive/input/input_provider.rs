use super::{
    compute_sessions_view, filter_models, push_toast, sort_sessions, AppState, Arc, Duration,
    KeyCode, KeyEvent, KeyModifiers, Modal, Screen, ToastVariant,
};

/// Handle a key event for the [`Modal::ModelSelect`] modal.
///
/// Extracted from `handle_modal_key` so each modal handler stays under the
/// 600-line limit.
pub(super) fn handle_model_select_key(
    state: &mut AppState,
    entries: Vec<String>,
    mut query: String,
    mut view: Vec<usize>,
    mut selected: usize,
    current_model: String,
    key: KeyEvent,
) {
    match key.code {
        KeyCode::Esc => return,
        KeyCode::Up => {
            selected = selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if !view.is_empty() {
                selected = (selected + 1).min(view.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => {
            selected = selected.saturating_sub(10);
        }
        KeyCode::PageDown => {
            if !view.is_empty() {
                selected = (selected + 10).min(view.len().saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            if let Some(model_id) = view.get(selected).and_then(|idx| entries.get(*idx)) {
                let new_model = model_id.clone();

                // 1. Update defaults so new sessions and footer fallback use new model
                state.defaults.model = new_model.clone();
                if let Some((provider, _)) = new_model.split_once('/') {
                    state.defaults.provider = provider.to_string();
                }

                // 2. Replace state.config so the next submit_prompt uses the new model.
                //    Also clear llm_provider so the model prefix ("openai/gpt-4o")
                //    drives provider selection instead of any explicit override.
                if let Some(config) = &state.config {
                    let mut updated = (**config).clone();
                    updated.model = new_model.clone();
                    updated.llm_provider = String::new();
                    // Rebuild the LLM client for the new provider so the engine
                    // routes requests to the correct API endpoint immediately.
                    if let Some(cell) = &state.llm_cell {
                        match rustcode_llm::build_client(&updated) {
                            Ok(new_client) => {
                                *cell.write().expect("llm cell lock not poisoned") = new_client;
                            }
                            Err(err) => {
                                push_toast(
                                    state,
                                    ToastVariant::Warning,
                                    format!("llm client: {err}"),
                                    Duration::from_secs(4),
                                );
                            }
                        }
                    }
                    state.config = Some(Arc::new(updated));
                }

                // 3. Persist the current session model (local backend) and
                // update in-memory session so the composer label stays stable.
                let current_session_id = match &state.screen {
                    Screen::Chat(chat) => Some(chat.session.id.clone()),
                    Screen::Sessions => None,
                };
                let mut persisted_session = None;
                if let Some(session_id) = current_session_id {
                    if let Ok(updated) = state
                        .backend
                        .update_session_model(&session_id, new_model.clone())
                    {
                        if let Some(idx) = state.sessions.iter().position(|s| s.id == updated.id) {
                            state.sessions[idx] = updated.clone();
                            sort_sessions(&mut state.sessions);
                            state.sessions_view =
                                compute_sessions_view(&state.sessions, &state.sessions_filter);
                            state.selected = state
                                .selected
                                .min(state.sessions_view.len().saturating_sub(1));
                        }
                        persisted_session = Some(updated);
                    }
                }
                if let Screen::Chat(ref mut chat) = state.screen {
                    if let Some(updated) = persisted_session {
                        chat.session = updated;
                    } else {
                        chat.session.model = new_model.clone();
                    }
                }

                push_toast(
                    state,
                    ToastVariant::Success,
                    format!("model: {new_model}"),
                    Duration::from_secs(3),
                );
            }
            return;
        }
        KeyCode::Backspace => {
            query.pop();
            view = filter_models(&entries, &query);
            selected = 0;
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                query.push(ch);
                view = filter_models(&entries, &query);
                selected = 0;
            }
        }
        _ => {}
    }
    state.modal = Some(Modal::ModelSelect {
        entries,
        query,
        view,
        selected,
        current_model,
    });
}
