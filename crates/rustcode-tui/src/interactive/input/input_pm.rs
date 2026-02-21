use super::{
    build_provider_entries, filter_provider_entries, provider_connect_methods,
    provider_display_name, provider_env_hint, push_toast, AppState, ConnectMethod, Duration,
    KeyCode, KeyEvent, KeyModifiers, Modal, ProviderManagerStep, ProviderOAuthDone,
    ProviderOAuthStarted, ToastVariant,
};

/// Handle a key event when the provider manager modal is open.
pub(super) fn handle_provider_manager_key(
    state: &mut AppState,
    step: ProviderManagerStep,
    key: KeyEvent,
) {
    match step {
        ProviderManagerStep::List {
            entries,
            mut query,
            mut view,
            mut selected,
        } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Down => {
                if !view.is_empty() {
                    selected = (selected + 1).min(view.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::PageUp => {
                selected = selected.saturating_sub(10);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::PageDown => {
                if !view.is_empty() {
                    selected = (selected + 10).min(view.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Enter => {
                let entry = view
                    .get(selected)
                    .and_then(|idx| entries.get(*idx))
                    .cloned();
                if let Some(entry) = entry {
                    let methods = provider_connect_methods(&entry.provider_id, entry.connected);
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::MethodSelect {
                            provider_id: entry.provider_id,
                            display_name: entry.display_name,
                            methods,
                            selected: 0,
                        },
                    });
                } else {
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::List {
                            entries,
                            query,
                            view,
                            selected,
                        },
                    });
                }
            }
            KeyCode::Backspace => {
                query.pop();
                view = filter_provider_entries(&entries, &query);
                selected = 0;
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    query.push(ch);
                    view = filter_provider_entries(&entries, &query);
                    selected = 0;
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query,
                        view,
                        selected,
                    },
                });
            }
        },

        ProviderManagerStep::MethodSelect {
            provider_id,
            display_name,
            methods,
            mut selected,
        } => match key.code {
            KeyCode::Esc => {
                // Go back to list
                let entries = build_provider_entries();
                let view = filter_provider_entries(&entries, "");
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::List {
                        entries,
                        query: String::new(),
                        view,
                        selected: 0,
                    },
                });
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
            KeyCode::Down => {
                if !methods.is_empty() {
                    selected = (selected + 1).min(methods.len().saturating_sub(1));
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
            KeyCode::Enter => {
                let method = methods.get(selected).copied();
                match method {
                    Some(ConnectMethod::ApiKey) => {
                        let env_hint = provider_env_hint(&provider_id);
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::ApiKeyInput {
                                provider_id,
                                display_name,
                                env_hint,
                                input: String::new(),
                                cursor: 0,
                            },
                        });
                    }
                    Some(ConnectMethod::OAuthDeviceCode) => {
                        handle_oauth_device_code(state, provider_id, display_name);
                    }
                    Some(ConnectMethod::Disconnect) => {
                        let store = rustcode_auth::AuthStore::open_default();
                        match store.remove(&provider_id) {
                            Ok(_) => push_toast(
                                state,
                                ToastVariant::Success,
                                format!("{display_name} disconnected"),
                                Duration::from_secs(3),
                            ),
                            Err(err) => push_toast(
                                state,
                                ToastVariant::Error,
                                format!("failed to disconnect: {err}"),
                                Duration::from_secs(4),
                            ),
                        }
                        // Return to refreshed list
                        let entries = build_provider_entries();
                        let view = filter_provider_entries(&entries, "");
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::List {
                                entries,
                                query: String::new(),
                                view,
                                selected: 0,
                            },
                        });
                    }
                    None => {
                        state.modal = Some(Modal::ProviderManager {
                            step: ProviderManagerStep::MethodSelect {
                                provider_id,
                                display_name,
                                methods,
                                selected,
                            },
                        });
                    }
                }
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name,
                        methods,
                        selected,
                    },
                });
            }
        },

        ProviderManagerStep::ApiKeyInput {
            provider_id,
            display_name,
            env_hint,
            mut input,
            mut cursor,
        } => match key.code {
            KeyCode::Esc => {
                // Go back to method select
                let methods = provider_connect_methods(&provider_id, false);
                let display = provider_display_name(&provider_id);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::MethodSelect {
                        provider_id,
                        display_name: display,
                        methods,
                        selected: 0,
                    },
                });
            }
            KeyCode::Enter => {
                if input.trim().is_empty() {
                    push_toast(
                        state,
                        ToastVariant::Warning,
                        "API key must not be empty",
                        Duration::from_secs(3),
                    );
                    state.modal = Some(Modal::ProviderManager {
                        step: ProviderManagerStep::ApiKeyInput {
                            provider_id,
                            display_name,
                            env_hint,
                            input,
                            cursor,
                        },
                    });
                } else {
                    let store = rustcode_auth::AuthStore::open_default();
                    match store.set_api_key(&provider_id, input.trim()) {
                        Ok(()) => {
                            push_toast(
                                state,
                                ToastVariant::Success,
                                format!("{display_name} connected!"),
                                Duration::from_secs(4),
                            );
                            // Close modal and re-open refreshed list
                            let entries = build_provider_entries();
                            let view = filter_provider_entries(&entries, "");
                            state.modal = Some(Modal::ProviderManager {
                                step: ProviderManagerStep::List {
                                    entries,
                                    query: String::new(),
                                    view,
                                    selected: 0,
                                },
                            });
                        }
                        Err(err) => {
                            push_toast(
                                state,
                                ToastVariant::Error,
                                format!("failed to save key: {err}"),
                                Duration::from_secs(5),
                            );
                            state.modal = Some(Modal::ProviderManager {
                                step: ProviderManagerStep::ApiKeyInput {
                                    provider_id,
                                    display_name,
                                    env_hint,
                                    input,
                                    cursor,
                                },
                            });
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if cursor > 0 {
                    let mut out = String::new();
                    for (idx, ch) in input.chars().enumerate() {
                        if idx + 1 != cursor {
                            out.push(ch);
                        }
                    }
                    input = out;
                    cursor -= 1;
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            KeyCode::Left => {
                cursor = cursor.saturating_sub(1);
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            KeyCode::Right => {
                cursor = (cursor + 1).min(input.chars().count());
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    let mut out = String::new();
                    let mut inserted = false;
                    for (idx, c) in input.chars().enumerate() {
                        if idx == cursor {
                            out.push(ch);
                            inserted = true;
                        }
                        out.push(c);
                    }
                    if !inserted {
                        out.push(ch);
                    }
                    input = out;
                    cursor += 1;
                }
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
            _ => {
                state.modal = Some(Modal::ProviderManager {
                    step: ProviderManagerStep::ApiKeyInput {
                        provider_id,
                        display_name,
                        env_hint,
                        input,
                        cursor,
                    },
                });
            }
        },

        // OAuthStarting / OAuthPending — only Esc cancels.
        ProviderManagerStep::OAuthStarting { .. } | ProviderManagerStep::OAuthPending { .. } => {
            if matches!(key.code, KeyCode::Esc) {
                // Drop channels by removing from AppState
                state.provider_oauth_start_rx = None;
                state.provider_oauth_done_rx = None;
                push_toast(
                    state,
                    ToastVariant::Info,
                    "OAuth cancelled",
                    Duration::from_secs(2),
                );
            } else {
                state.modal = Some(Modal::ProviderManager { step });
            }
        }
    }
}

/// Spawn the OAuth device-code flow in the background and transition the modal
/// to the `OAuthStarting` step.
fn handle_oauth_device_code(state: &mut AppState, provider_id: String, display_name: String) {
    let (start_tx, start_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<ProviderOAuthStarted, String>>();
    let (done_tx, done_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<ProviderOAuthDone, String>>();
    state.provider_oauth_start_rx = Some(start_rx);
    state.provider_oauth_done_rx = Some(done_rx);

    let provider_id_clone = provider_id.clone();
    tokio::spawn(async move {
        let flow = match rustcode_auth::start_device_code_flow(&provider_id_clone, None).await {
            Ok(f) => f,
            Err(err) => {
                let _ = start_tx.send(Err(err.to_string()));
                return;
            }
        };

        let _ = start_tx.send(Ok(ProviderOAuthStarted {
            provider_id: provider_id_clone.clone(),
            verification_uri: flow.verification_uri.clone(),
            user_code: flow.user_code.clone(),
        }));

        let timeout = std::time::Duration::from_secs(flow.expires_in_secs);
        let cred = match rustcode_auth::poll_device_code_flow_for_credential(&flow, timeout).await {
            Ok(c) => c,
            Err(err) => {
                let _ = done_tx.send(Err(err.to_string()));
                return;
            }
        };

        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let expires_at_unix = cred.expires_in_secs.map(|secs| now_unix + secs as i64);

        let _ = done_tx.send(Ok(ProviderOAuthDone {
            provider_id: provider_id_clone,
            access_token: cred.access_token,
            refresh_token: cred.refresh_token,
            expires_at_unix,
            account_id: cred.account_id,
        }));
    });

    state.modal = Some(Modal::ProviderManager {
        step: ProviderManagerStep::OAuthStarting {
            provider_id,
            display_name,
        },
    });
}
