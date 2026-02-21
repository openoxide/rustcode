//! Frame scheduling with coalescing and rate limiting.
//!
//! Inspired by codex-rs `frame_requester.rs`.  Multiple draw requests arriving
//! within the same frame interval are coalesced into a single draw signal,
//! preventing excessive rendering while maintaining smooth animation (~120 FPS cap).

use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};

/// Minimum interval between frames (~120 FPS).
const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(8_333);

/// Cloneable handle used to request a frame draw.
#[derive(Clone)]
pub(crate) struct FrameRequester {
    tx: mpsc::UnboundedSender<Instant>,
}

impl FrameRequester {
    /// Request a frame to be drawn as soon as the rate limiter allows.
    pub(crate) fn schedule_frame(&self) {
        let _ = self.tx.send(Instant::now());
    }

    /// Request a frame after `delay`.
    pub(crate) fn schedule_frame_in(&self, delay: Duration) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(Instant::now());
        });
    }
}

/// Background task that coalesces draw requests and emits at most one draw
/// signal per [`MIN_FRAME_INTERVAL`].
pub(crate) struct FrameScheduler {
    rx: mpsc::UnboundedReceiver<Instant>,
    draw_tx: broadcast::Sender<()>,
}

impl FrameScheduler {
    /// Create a new scheduler and its associated requester + draw receiver.
    ///
    /// Returns `(requester, draw_receiver, scheduler)`.  Call
    /// [`FrameScheduler::run`] as a spawned tokio task.
    pub(crate) fn new() -> (FrameRequester, broadcast::Receiver<()>, Self) {
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (draw_tx, draw_rx) = broadcast::channel(4);
        let requester = FrameRequester { tx: req_tx };
        let scheduler = Self {
            rx: req_rx,
            draw_tx,
        };
        (requester, draw_rx, scheduler)
    }

    /// Run the scheduler loop.  Exits when all [`FrameRequester`] handles are dropped.
    pub(crate) async fn run(mut self) {
        let mut last_draw = Instant::now() - MIN_FRAME_INTERVAL;

        while let Some(_requested_at) = self.rx.recv().await {
            // Drain any additional coalesced requests.
            while self.rx.try_recv().is_ok() {}

            // Enforce rate limit.
            let elapsed = last_draw.elapsed();
            if elapsed < MIN_FRAME_INTERVAL {
                tokio::time::sleep(MIN_FRAME_INTERVAL - elapsed).await;
                // Drain again after sleep in case more requests arrived.
                while self.rx.try_recv().is_ok() {}
            }

            last_draw = Instant::now();
            // If all receivers are gone, the send returns Err but we continue
            // accepting requests in case new receivers subscribe later.
            let _ = self.draw_tx.send(());
        }
    }
}

/// Create a no-op [`FrameRequester`] for tests that don't need frame scheduling.
#[cfg(test)]
pub(crate) fn test_frame_requester() -> FrameRequester {
    let (tx, _rx) = mpsc::unbounded_channel();
    FrameRequester { tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requester_is_clone_and_send() {
        let (req, _draw_rx, _sched) = FrameScheduler::new();
        let req2 = req.clone();
        req.schedule_frame();
        req2.schedule_frame();
    }

    #[tokio::test]
    async fn scheduler_emits_draw_signal() {
        let (req, mut draw_rx, scheduler) = FrameScheduler::new();

        let handle = tokio::spawn(scheduler.run());

        req.schedule_frame();
        let result = tokio::time::timeout(Duration::from_millis(100), draw_rx.recv()).await;
        assert!(result.is_ok(), "should receive draw signal");

        drop(req);
        let _ = tokio::time::timeout(Duration::from_millis(100), handle).await;
    }

    #[tokio::test]
    async fn coalesces_multiple_requests() {
        let (req, mut draw_rx, scheduler) = FrameScheduler::new();

        let handle = tokio::spawn(scheduler.run());

        // Send many requests rapidly.
        for _ in 0..50 {
            req.schedule_frame();
        }

        // Should get at most a few draw signals (coalesced), not 50.
        let mut count = 0;
        while tokio::time::timeout(Duration::from_millis(50), draw_rx.recv())
            .await
            .is_ok()
        {
            count += 1;
        }
        assert!(count < 10, "expected coalescing, got {count} signals");

        drop(req);
        let _ = tokio::time::timeout(Duration::from_millis(100), handle).await;
    }
}
