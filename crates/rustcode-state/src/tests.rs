use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rustcode_core::session::{MessageRole, StoredMessage, StoredToolCall};

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time must be monotonic")
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("rustcode-state-{name}-{pid}-{now}"));
    fs::create_dir_all(&dir).expect("must create temp dir");
    dir
}

#[test]
fn create_list_append_and_load_round_trip() {
    let root = temp_dir("roundtrip");
    let store = SessionStore::with_root(root.join("sessions"));

    let session = store
        .create_session(
            Some("t1".to_string()),
            None,
            Path::new("/tmp"),
            Path::new("/tmp"),
            "openai/gpt-test",
        )
        .expect("create session");

    let sessions = store.list_sessions().expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session.id);

    let msg = StoredMessage {
        id: store.new_message_id(),
        role: MessageRole::User,
        created_at_unix_ms: now_unix_ms().expect("now"),
        content: serde_json::Value::String("hello".to_string()),
        reasoning: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: vec![StoredToolCall {
            id: "call_1".to_string(),
            name: "list".to_string(),
            arguments: "{}".to_string(),
        }],
    };
    store
        .append_message(&session.id, &msg)
        .expect("append message");

    let loaded = store.load_messages(&session.id).expect("load messages");
    assert_eq!(loaded, vec![msg]);
}

#[test]
fn fork_clones_messages_and_sets_parent() {
    let root = temp_dir("fork");
    let store = SessionStore::with_root(root.join("sessions"));

    let session = store
        .create_session(None, None, Path::new("/tmp"), Path::new("/tmp"), "m")
        .expect("create session");

    let msg = StoredMessage {
        id: store.new_message_id(),
        role: MessageRole::Assistant,
        created_at_unix_ms: now_unix_ms().expect("now"),
        content: serde_json::Value::String("hi".to_string()),
        reasoning: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    };
    store
        .append_message(&session.id, &msg)
        .expect("append message");

    let forked = store
        .fork_session(&session.id, Some("forked".to_string()))
        .expect("fork");
    assert_eq!(forked.parent_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(forked.title.as_deref(), Some("forked"));

    let loaded = store.load_messages(&forked.id).expect("load");
    assert_eq!(loaded, vec![msg]);
}

#[test]
fn load_unknown_session_is_not_found() {
    let root = temp_dir("not-found");
    let store = SessionStore::with_root(root.join("sessions"));
    let err = store
        .load_messages("does-not-exist")
        .expect_err("must fail");
    assert!(matches!(err, StateError::NotFound(_)));
}

#[test]
fn update_session_title_round_trips() {
    let root = temp_dir("rename");
    let store = SessionStore::with_root(root.join("sessions"));
    let session = store
        .create_session(None, None, Path::new("/tmp"), Path::new("/tmp"), "m")
        .expect("create");

    let updated = store
        .update_session_title(&session.id, Some("new title".to_string()))
        .expect("update title");
    assert_eq!(updated.title.as_deref(), Some("new title"));

    let loaded = store.get_session(&session.id).expect("get");
    assert_eq!(loaded.title.as_deref(), Some("new title"));
}

#[test]
fn delete_session_removes_from_list_and_load_fails() {
    let root = temp_dir("delete");
    let store = SessionStore::with_root(root.join("sessions"));
    let session = store
        .create_session(None, None, Path::new("/tmp"), Path::new("/tmp"), "m")
        .expect("create");

    store.delete_session(&session.id).expect("delete");
    let sessions = store.list_sessions().expect("list");
    assert!(sessions.is_empty());

    let err = store
        .load_messages(&session.id)
        .expect_err("load must fail");
    assert!(matches!(err, StateError::NotFound(_)));
}

#[test]
fn session_ids_use_rc_prefix_with_7_chars() {
    let root = temp_dir("id-format");
    let store = SessionStore::with_root(root.join("sessions"));
    let session = store
        .create_session(None, None, Path::new("/tmp"), Path::new("/tmp"), "m")
        .expect("create");

    assert!(session.id.starts_with("rc-"));
    assert_eq!(session.id.len(), 10);
    assert!(session.id["rc-".len()..]
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()));
}

#[test]
fn update_session_model_round_trips() {
    let root = temp_dir("update-model");
    let store = SessionStore::with_root(root.join("sessions"));
    let session = store
        .create_session(
            None,
            None,
            Path::new("/tmp"),
            Path::new("/tmp"),
            "openai/gpt-5",
        )
        .expect("create");

    let updated = store
        .update_session_model(&session.id, "openai/gpt-5.3-codex")
        .expect("update model");
    assert_eq!(updated.model, "openai/gpt-5.3-codex");

    let loaded = store.get_session(&session.id).expect("get");
    assert_eq!(loaded.model, "openai/gpt-5.3-codex");
}

#[test]
fn create_session_stores_empty_branch_outside_git_repo() {
    let root = temp_dir("branch-empty");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let store = SessionStore::with_root(root.join("sessions"));

    let session = store
        .create_session(
            Some("t1".to_string()),
            None,
            &workspace,
            &workspace,
            "openai/gpt-test",
        )
        .expect("create session");

    assert_eq!(session.branch, "");
}

#[test]
fn create_session_stores_git_branch_name() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = temp_dir("branch-git");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");

    let init = Command::new("git")
        .args(["init", "-b", "session-branch-test"])
        .current_dir(&workspace)
        .output();
    let init_ok = init.as_ref().is_ok_and(|out| out.status.success());
    if !init_ok {
        let fallback = Command::new("git")
            .arg("init")
            .current_dir(&workspace)
            .output()
            .expect("git init fallback");
        assert!(fallback.status.success(), "git init must succeed");

        let checkout = Command::new("git")
            .args(["checkout", "-b", "session-branch-test"])
            .current_dir(&workspace)
            .output()
            .expect("git checkout -b");
        assert!(checkout.status.success(), "git checkout -b must succeed");
    }

    let store = SessionStore::with_root(root.join("sessions"));
    let session = store
        .create_session(
            Some("t1".to_string()),
            None,
            &workspace,
            &workspace,
            "openai/gpt-test",
        )
        .expect("create session");

    assert_eq!(session.branch, "session-branch-test");
}

#[test]
fn push_prompt_history_entry_dedupes_and_trims() {
    let mut history = Vec::new();
    assert!(!push_prompt_history_entry(&mut history, "   "));
    assert!(push_prompt_history_entry(&mut history, "  first  "));
    assert_eq!(history, vec!["first"]);
    assert!(!push_prompt_history_entry(&mut history, "first"));
    assert!(push_prompt_history_entry(&mut history, "second"));
    assert_eq!(history, vec!["first", "second"]);
}

#[test]
fn prompt_history_store_round_trip_and_cap() {
    let root = temp_dir("prompt-history");
    let path = root.join("prompt-history.json");
    let store = PromptHistoryStore::with_path(path);

    for i in 0..(PROMPT_HISTORY_LIMIT + 20) {
        let prompt = format!("prompt-{i}");
        assert!(store.append(&prompt).expect("append prompt"));
    }
    assert!(!store.append("prompt-219").expect("append dedupe"));

    let loaded = store.load().expect("load prompt history");
    assert_eq!(loaded.len(), PROMPT_HISTORY_LIMIT);
    assert_eq!(loaded.first().expect("first"), "prompt-20");
    assert_eq!(loaded.last().expect("last"), "prompt-219");
}
