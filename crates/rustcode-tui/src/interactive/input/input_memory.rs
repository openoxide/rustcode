use super::{push_toast, AppState, Duration, KeyCode, KeyEvent, Modal, ToastVariant};

/// Handle key events for the memory viewer and memory clear confirmation modals.
///
/// Returns `true` if the modal should remain open (caller should NOT clear `state.modal`).
/// Returns `false` if the modal was dismissed.
pub(super) fn handle_memory_key(state: &mut AppState, modal: Modal, key: KeyEvent) {
    match modal {
        Modal::MemoryViewer {
            content,
            raw_count,
            updated_at,
            enabled,
            mut scroll,
            total_lines,
        } => {
            match key.code {
                KeyCode::Esc => return,
                KeyCode::Up | KeyCode::Char('k') => {
                    scroll = scroll.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    scroll = (scroll + 1).min(total_lines.saturating_sub(1));
                }
                KeyCode::PageUp => {
                    scroll = scroll.saturating_sub(20);
                }
                KeyCode::PageDown => {
                    scroll = (scroll + 20).min(total_lines.saturating_sub(1));
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    scroll = 0;
                }
                KeyCode::End | KeyCode::Char('G') => {
                    scroll = total_lines.saturating_sub(1);
                }
                KeyCode::Char('t') => {
                    let new_enabled = !enabled;
                    if let Some(storage) = rustcode_memories::MemoryStorage::new() {
                        let _ = storage.set_enabled(new_enabled);
                    }
                    push_toast(
                        state,
                        if new_enabled {
                            ToastVariant::Success
                        } else {
                            ToastVariant::Warning
                        },
                        if new_enabled {
                            "memory: enabled"
                        } else {
                            "memory: disabled"
                        },
                        Duration::from_secs(3),
                    );
                    state.modal = Some(Modal::MemoryViewer {
                        content,
                        raw_count,
                        updated_at,
                        enabled: new_enabled,
                        scroll,
                        total_lines,
                    });
                    return;
                }
                _ => {}
            }
            state.modal = Some(Modal::MemoryViewer {
                content,
                raw_count,
                updated_at,
                enabled,
                scroll,
                total_lines,
            });
        }
        Modal::MemoryClearConfirm => match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {}
            KeyCode::Char('y') => match rustcode_memories::MemoryStorage::new() {
                Some(storage) => match storage.clear_all() {
                    Ok(()) => push_toast(
                        state,
                        ToastVariant::Success,
                        "all memories cleared",
                        Duration::from_secs(3),
                    ),
                    Err(err) => push_toast(
                        state,
                        ToastVariant::Error,
                        format!("failed to clear memories: {err}"),
                        Duration::from_secs(4),
                    ),
                },
                None => push_toast(
                    state,
                    ToastVariant::Warning,
                    "memory storage unavailable",
                    Duration::from_secs(3),
                ),
            },
            _ => {
                state.modal = Some(Modal::MemoryClearConfirm);
            }
        },
        _ => {}
    }
}
