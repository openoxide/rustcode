//! Context window tracking for the agent loop.
//!
//! Tracks cumulative token usage across LLM steps and detects when the
//! context window is approaching its limit. Exposes a `usage_percent()`
//! for TUI display.

use rustcode_llm::TokenUsage;

/// Default buffer of tokens to reserve before declaring overflow.
/// This leaves room for the model's output and avoids hard cutoffs.
const DEFAULT_OVERFLOW_BUFFER: u64 = 20_000;

/// Known context window sizes for common models.
/// Delegates to the centralized model registry in `rustcode_llm`.
fn model_context_limit(model: &str) -> u64 {
    rustcode_llm::model_registry::context_limit(model)
}

/// Simple token estimator based on character count.
///
/// Uses the common heuristic of ~4 characters per token.
/// This is a rough estimate; actual tokenization varies by model.
#[must_use]
pub fn token_estimate(text: &str) -> u64 {
    let chars = text.len() as u64;
    // ~4 chars per token on average for English text
    chars.div_ceil(4)
}

/// Tracks cumulative token usage across LLM steps and detects overflow.
#[derive(Debug)]
pub struct ContextTracker {
    /// The model being used (for context limit lookup).
    model: String,
    /// Context window size in tokens.
    context_limit: u64,
    /// Buffer to reserve before declaring overflow.
    overflow_buffer: u64,
    /// Cumulative input tokens across all steps.
    total_input: u64,
    /// Cumulative output tokens across all steps.
    total_output: u64,
    /// Most recent token count from the last step (best estimate of current context size).
    last_total: u64,
    /// Number of LLM steps completed.
    step_count: u64,
}

impl ContextTracker {
    /// Create a new tracker for the given model.
    #[must_use]
    pub fn new(model: &str) -> Self {
        let context_limit = model_context_limit(model);
        Self {
            model: model.to_string(),
            context_limit,
            overflow_buffer: DEFAULT_OVERFLOW_BUFFER,
            total_input: 0,
            total_output: 0,
            last_total: 0,
            step_count: 0,
        }
    }

    /// Create a tracker with a custom context limit.
    #[must_use]
    pub fn with_limit(model: &str, limit: u64) -> Self {
        Self {
            model: model.to_string(),
            context_limit: limit,
            overflow_buffer: DEFAULT_OVERFLOW_BUFFER,
            total_input: 0,
            total_output: 0,
            last_total: 0,
            step_count: 0,
        }
    }

    /// Record token usage from a single LLM step.
    pub fn record_usage(&mut self, usage: &TokenUsage) {
        self.total_input += usage.input;
        self.total_output += usage.output;
        self.last_total = usage.total;
        self.step_count += 1;
    }

    /// Returns true when the context window is approaching its limit.
    ///
    /// Uses the last step's total token count as the best estimate of
    /// current context size, and triggers when remaining headroom is
    /// less than the overflow buffer.
    #[must_use]
    pub fn is_overflow(&self) -> bool {
        if self.context_limit == 0 || self.last_total == 0 {
            return false;
        }
        let usable = self.context_limit.saturating_sub(self.overflow_buffer);
        self.last_total >= usable
    }

    /// Returns the context window usage as a percentage (0.0–100.0).
    ///
    /// This value is suitable for display in the TUI status bar.
    #[must_use]
    pub fn usage_percent(&self) -> f64 {
        if self.context_limit == 0 {
            return 0.0;
        }
        let pct = (self.last_total as f64 / self.context_limit as f64) * 100.0;
        pct.min(100.0)
    }

    /// Returns the configured context limit in tokens.
    #[must_use]
    pub fn context_limit(&self) -> u64 {
        self.context_limit
    }

    /// Returns the most recent total token count (current context size estimate).
    #[must_use]
    pub fn current_tokens(&self) -> u64 {
        self.last_total
    }

    /// Returns the remaining tokens before overflow.
    #[must_use]
    pub fn remaining_tokens(&self) -> u64 {
        self.context_limit.saturating_sub(self.last_total)
    }

    /// Returns cumulative input tokens across all steps.
    #[must_use]
    pub fn cumulative_input(&self) -> u64 {
        self.total_input
    }

    /// Returns cumulative output tokens across all steps.
    #[must_use]
    pub fn cumulative_output(&self) -> u64 {
        self.total_output
    }

    /// Returns the number of completed LLM steps.
    #[must_use]
    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Returns the model name.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Format a compact status string for TUI display.
    /// Example: "42.1% (84K/200K tokens, step 5)"
    #[must_use]
    pub fn status_line(&self) -> String {
        let pct = self.usage_percent();
        let current_k = self.last_total / 1000;
        let limit_k = self.context_limit / 1000;
        format!(
            "{pct:.1}% ({current_k}K/{limit_k}K tokens, step {})",
            self.step_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_limits_known_models() {
        assert_eq!(model_context_limit("claude-3.5-sonnet"), 200_000);
        assert_eq!(model_context_limit("gpt-4o"), 128_000);
        assert_eq!(model_context_limit("gemini-2.0-flash"), 1_000_000);
        assert_eq!(model_context_limit("deepseek-chat"), 64_000);
    }

    #[test]
    fn model_limits_unknown_model() {
        assert_eq!(model_context_limit("some-unknown-model-v99"), 128_000);
    }

    #[test]
    fn token_estimate_basic() {
        // 12 chars → ~3 tokens
        assert_eq!(token_estimate("hello world!"), 3);
        // empty → 0
        assert_eq!(token_estimate(""), 0);
    }

    #[test]
    fn tracker_no_overflow_initially() {
        let tracker = ContextTracker::new("claude-3.5-sonnet");
        assert!(!tracker.is_overflow());
        assert!(tracker.usage_percent().abs() < f64::EPSILON);
        assert_eq!(tracker.step_count(), 0);
    }

    #[test]
    fn tracker_records_usage() {
        let mut tracker = ContextTracker::with_limit("test-model", 100_000);
        tracker.record_usage(&TokenUsage {
            input: 5000,
            output: 1000,
            total: 6000,
            cache_read: 0,
            cache_write: 0,
        });
        assert_eq!(tracker.step_count(), 1);
        assert_eq!(tracker.current_tokens(), 6000);
        assert_eq!(tracker.cumulative_input(), 5000);
        assert_eq!(tracker.cumulative_output(), 1000);
    }

    #[test]
    fn tracker_detects_overflow() {
        let mut tracker = ContextTracker::with_limit("test-model", 100_000);
        // 85K total — within buffer (100K - 20K = 80K threshold)
        tracker.record_usage(&TokenUsage {
            input: 80000,
            output: 5000,
            total: 85_000,
            cache_read: 0,
            cache_write: 0,
        });
        assert!(tracker.is_overflow());
    }

    #[test]
    fn tracker_no_overflow_below_threshold() {
        let mut tracker = ContextTracker::with_limit("test-model", 100_000);
        // 50K total — well below 80K threshold
        tracker.record_usage(&TokenUsage {
            input: 45000,
            output: 5000,
            total: 50_000,
            cache_read: 0,
            cache_write: 0,
        });
        assert!(!tracker.is_overflow());
    }

    #[test]
    fn tracker_usage_percent() {
        let mut tracker = ContextTracker::with_limit("test-model", 200_000);
        tracker.record_usage(&TokenUsage {
            input: 0,
            output: 0,
            total: 100_000,
            cache_read: 0,
            cache_write: 0,
        });
        assert!((tracker.usage_percent() - 50.0).abs() < 0.01);
    }

    #[test]
    fn tracker_status_line() {
        let mut tracker = ContextTracker::with_limit("test-model", 200_000);
        tracker.record_usage(&TokenUsage {
            input: 40000,
            output: 10000,
            total: 50_000,
            cache_read: 0,
            cache_write: 0,
        });
        let line = tracker.status_line();
        assert!(line.contains("25.0%"));
        assert!(line.contains("50K/200K"));
    }

    #[test]
    fn tracker_remaining_tokens() {
        let mut tracker = ContextTracker::with_limit("test-model", 100_000);
        tracker.record_usage(&TokenUsage {
            input: 0,
            output: 0,
            total: 60_000,
            cache_read: 0,
            cache_write: 0,
        });
        assert_eq!(tracker.remaining_tokens(), 40_000);
    }
}
