// Integration tests for M4 session management features

use super::*;
use crate::{instructions, retry, session_summary, system_prompt};

#[test]
fn instructions_load_from_workspace() {
    let temp_dir = std::env::temp_dir().join("rustcode_test_instructions");
    std::fs::create_dir_all(&temp_dir).ok();

    // Create RUSTCODE.md
    let rustcode_path = temp_dir.join("RUSTCODE.md");
    std::fs::write(&rustcode_path, "# Test Instructions\n\nThis is a test.").unwrap();

    let loaded = instructions::load_instructions(&temp_dir);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].content.contains("Test Instructions"));

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn instructions_prefer_rustcode_over_agents() {
    let temp_dir = std::env::temp_dir().join("rustcode_test_precedence");
    std::fs::create_dir_all(&temp_dir).ok();

    // Create both files
    std::fs::write(temp_dir.join("RUSTCODE.md"), "# RUSTCODE").unwrap();
    std::fs::write(temp_dir.join("AGENTS.md"), "# AGENTS").unwrap();

    let loaded = instructions::load_instructions(&temp_dir);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].content.contains("RUSTCODE"));
    assert!(!loaded[0].content.contains("AGENTS"));

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn system_prompt_includes_all_components() {
    let temp_dir = std::env::temp_dir().join("rustcode_test_sysprompt");
    std::fs::create_dir_all(&temp_dir).ok();

    // Create instruction file
    std::fs::write(temp_dir.join("RUSTCODE.md"), "# Custom Instructions").unwrap();

    let prompt =
        system_prompt::build_system_prompt("claude-3.5-sonnet", &temp_dir, true, &[], &[], None);

    // Check all components are present
    assert!(prompt.contains("You are rustcode"));
    assert!(prompt.contains("Claude model"));
    assert!(prompt.contains("<environment>"));
    assert!(prompt.contains("Custom Instructions"));

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn system_prompt_model_hints() {
    let temp = std::env::temp_dir();

    let claude_prompt =
        system_prompt::build_system_prompt("claude-3.5-sonnet", &temp, false, &[], &[], None);
    assert!(claude_prompt.contains("Claude model"));

    let gpt_prompt = system_prompt::build_system_prompt("gpt-4o", &temp, false, &[], &[], None);
    assert!(gpt_prompt.contains("OpenAI model"));

    let gemini_prompt =
        system_prompt::build_system_prompt("gemini-2.0-flash", &temp, false, &[], &[], None);
    assert!(gemini_prompt.contains("Gemini model"));
}

#[test]
fn retry_policy_delays() {
    let policy = retry::RetryPolicy::default();

    // Check exponential backoff
    let delay1 = policy.delay(1);
    let delay2 = policy.delay(2);
    let delay3 = policy.delay(3);

    assert_eq!(delay1.as_millis(), 2000); // 2s
    assert_eq!(delay2.as_millis(), 4000); // 4s
    assert_eq!(delay3.as_millis(), 8000); // 8s
}

#[test]
fn retry_identifies_retryable_errors() {
    assert!(retry::is_retryable("rate_limit exceeded"));
    assert!(retry::is_retryable("429 Too Many Requests"));
    assert!(retry::is_retryable("server overloaded"));
    assert!(retry::is_retryable("503 Service Unavailable"));
    assert!(retry::is_retryable("connection timeout"));

    assert!(!retry::is_retryable("401 Unauthorized"));
    assert!(!retry::is_retryable("invalid api key"));
    assert!(!retry::is_retryable("context overflow"));
    assert!(!retry::is_retryable("content_policy violation"));
}

#[tokio::test]
async fn retry_stops_after_max_attempts() {
    let policy = retry::RetryPolicy {
        max_retries: 2,
        initial_delay_ms: 10,
        backoff_factor: 2,
        max_delay_ms: 100,
    };

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let result = retry::retry_llm_call(&policy, || {
        let count = call_count_clone.clone();
        async move {
            let mut c = count.lock().await;
            *c += 1;
            Err::<(), String>("503 Service Unavailable".to_string())
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(*call_count.lock().await, 3); // Initial + 2 retries
}

#[tokio::test]
async fn retry_succeeds_on_eventual_success() {
    let policy = retry::RetryPolicy::default();

    let call_count = Arc::new(Mutex::new(0));
    let call_count_clone = call_count.clone();

    let result = retry::retry_llm_call(&policy, || {
        let count = call_count_clone.clone();
        async move {
            let mut c = count.lock().await;
            *c += 1;
            let current = *c;
            drop(c);

            if current < 3 {
                Err::<i32, String>("503 Service Unavailable".to_string())
            } else {
                Ok(42)
            }
        }
    })
    .await;

    assert_eq!(result.unwrap(), 42);
    assert_eq!(*call_count.lock().await, 3);
}

#[test]
fn session_summary_formats_correctly() {
    let summary = session_summary::SessionSummary {
        files_changed: 3,
        additions: 42,
        deletions: 17,
    };

    let display = summary.to_display();
    assert!(display.contains("3 files"));
    assert!(display.contains("42 insertions"));
    assert!(display.contains("17 deletions"));
}

#[test]
fn session_summary_handles_singular() {
    let summary = session_summary::SessionSummary {
        files_changed: 1,
        additions: 1,
        deletions: 1,
    };

    let display = summary.to_display();
    assert!(display.contains("1 file changed"));
    assert!(!display.contains("files"));
    assert!(display.contains("1 insertion"));
    assert!(!display.contains("insertions"));
}

#[test]
fn session_summary_no_changes() {
    let summary = session_summary::SessionSummary::default();
    assert!(!summary.has_changes());
    assert_eq!(summary.to_display(), "No files changed");
}
