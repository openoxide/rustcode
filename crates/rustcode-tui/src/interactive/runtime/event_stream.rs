//! Async event stream that merges crossterm terminal events with draw requests.
//!
//! Inspired by codex-rs `event_stream.rs` — combines terminal input with
//! frame-request signals into a single `Stream<Item = TuiEvent>` consumed by
//! the main `tokio::select!` loop.

use std::pin::Pin;
use std::task::{Context, Poll};

use crossterm::event::{Event as CtEvent, EventStream, KeyEventKind};
use futures_util::Stream;
use tokio_stream::wrappers::BroadcastStream;

/// Events consumed by the TUI main loop.
#[derive(Debug)]
pub(crate) enum TuiEvent {
    /// A key press (already filtered to `KeyEventKind::Press`).
    Key(crossterm::event::KeyEvent),
    /// Bracketed-paste text.
    Paste(String),
    /// Mouse event (scroll, click, etc.).
    Mouse(crossterm::event::MouseEvent),
    /// A frame should be drawn (coalesced by the frame scheduler).
    Draw,
}

/// Wraps crossterm's `EventStream` and a draw-request broadcast channel into
/// a single async `Stream<Item = TuiEvent>`.
///
/// Uses round-robin polling so neither source can starve the other.
pub(crate) struct TuiEventStream {
    crossterm: EventStream,
    draw_rx: BroadcastStream<()>,
    /// Toggle for round-robin fairness: `true` = poll draw first this tick.
    poll_draw_first: bool,
}

impl TuiEventStream {
    /// Create a new event stream.
    ///
    /// `draw_rx` receives `()` signals from the [`super::frame_scheduler::FrameScheduler`]
    /// whenever a frame is due.
    pub(crate) fn new(draw_rx: tokio::sync::broadcast::Receiver<()>) -> Self {
        Self {
            crossterm: EventStream::new(),
            draw_rx: BroadcastStream::new(draw_rx),
            poll_draw_first: false,
        }
    }
}

/// Map a raw crossterm event to a `TuiEvent`, filtering out non-press key events.
fn map_crossterm_event(event: CtEvent) -> Option<TuiEvent> {
    match event {
        CtEvent::Key(key) => {
            if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                Some(TuiEvent::Key(key))
            } else {
                None
            }
        }
        CtEvent::Paste(text) => Some(TuiEvent::Paste(text)),
        CtEvent::Mouse(mouse) => Some(TuiEvent::Mouse(mouse)),
        CtEvent::Resize(..) => Some(TuiEvent::Draw),
        CtEvent::FocusGained | CtEvent::FocusLost => None,
    }
}

impl Stream for TuiEventStream {
    type Item = TuiEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Round-robin: alternate which source we poll first each invocation.
        let (first_poll, second_poll) = if this.poll_draw_first {
            (PollSource::Draw, PollSource::Crossterm)
        } else {
            (PollSource::Crossterm, PollSource::Draw)
        };
        this.poll_draw_first = !this.poll_draw_first;

        for source in [first_poll, second_poll] {
            match source {
                PollSource::Draw => {
                    if let Poll::Ready(Some(_)) = Pin::new(&mut this.draw_rx).poll_next(cx) {
                        return Poll::Ready(Some(TuiEvent::Draw));
                    }
                }
                PollSource::Crossterm => {
                    if let Poll::Ready(maybe) = Pin::new(&mut this.crossterm).poll_next(cx) {
                        match maybe {
                            Some(Ok(ct_event)) => {
                                if let Some(tui_event) = map_crossterm_event(ct_event) {
                                    return Poll::Ready(Some(tui_event));
                                }
                                // Filtered event — fall through to next source.
                            }
                            Some(Err(_)) | None => {
                                // Crossterm stream ended or errored — terminal is gone.
                                return Poll::Ready(None);
                            }
                        }
                    }
                }
            }
        }

        Poll::Pending
    }
}

#[derive(Clone, Copy)]
enum PollSource {
    Draw,
    Crossterm,
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;

    #[test]
    fn map_filters_release_events() {
        let release = CtEvent::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: crossterm::event::KeyEventState::NONE,
        });
        assert!(map_crossterm_event(release).is_none());
    }

    #[test]
    fn map_converts_press_events() {
        let press = CtEvent::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        });
        assert!(matches!(map_crossterm_event(press), Some(TuiEvent::Key(_))));
    }

    #[test]
    fn map_converts_paste() {
        let paste = CtEvent::Paste("hello".to_string());
        assert!(matches!(
            map_crossterm_event(paste),
            Some(TuiEvent::Paste(_))
        ));
    }

    #[test]
    fn map_converts_resize_to_draw() {
        let resize = CtEvent::Resize(80, 24);
        assert!(matches!(map_crossterm_event(resize), Some(TuiEvent::Draw)));
    }

    #[test]
    fn map_converts_repeat_events() {
        let repeat = CtEvent::Key(KeyEvent {
            code: KeyCode::Backspace,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Repeat,
            state: crossterm::event::KeyEventState::NONE,
        });
        assert!(matches!(
            map_crossterm_event(repeat),
            Some(TuiEvent::Key(_))
        ));
    }

    #[test]
    fn map_filters_focus_events() {
        assert!(map_crossterm_event(CtEvent::FocusGained).is_none());
        assert!(map_crossterm_event(CtEvent::FocusLost).is_none());
    }
}
