use super::environment::build_environment_block;
use super::*;

fn mock_tools() -> Vec<rustcode_llm::ToolSpec> {
    vec![
        rustcode_llm::ToolSpec {
            name: "read".to_string(),
            description: "Read file contents".to_string(),
            parameters: serde_json::json!({}),
        },
        rustcode_llm::ToolSpec {
            name: "write".to_string(),
            description: "Write content to a file".to_string(),
            parameters: serde_json::json!({}),
        },
        rustcode_llm::ToolSpec {
            name: "bash".to_string(),
            description: "Execute a shell command".to_string(),
            parameters: serde_json::json!({}),
        },
    ]
}

#[test]
fn build_system_prompt_contains_identity() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("You are rustcode"));
    assert!(prompt.contains("production-grade"));
}

#[test]
fn build_system_prompt_contains_editing_constraints() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("Editing Constraints"));
    assert!(prompt.contains("apply_patch"));
}

#[test]
fn build_system_prompt_contains_tool_usage_policy() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("Tool Usage Policy"));
}

#[test]
fn build_system_prompt_contains_git_hygiene() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("Git & Workspace Hygiene"));
    assert!(prompt.contains("NEVER revert existing changes"));
}

#[test]
fn build_system_prompt_includes_environment() {
    let prompt = build_system_prompt(
        "claude-3.5-sonnet",
        Path::new("/project"),
        true,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("<environment>"));
    assert!(prompt.contains("claude-3.5-sonnet"));
    assert!(prompt.contains("/project"));
    assert!(prompt.contains("Git repository: yes"));
}

#[test]
fn build_system_prompt_includes_tool_summary() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        None,
    );
    assert!(prompt.contains("<tools>"));
    assert!(prompt.contains("**read**"));
    assert!(prompt.contains("**write**"));
    assert!(prompt.contains("**bash**"));
    assert!(prompt.contains("</tools>"));
}

#[test]
fn build_system_prompt_no_tools() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &[],
        &[],
        None,
        None,
    );
    assert!(!prompt.contains("<tools>"));
}

#[test]
fn model_hints_claude() {
    let hints = model_hints("claude-3.5-sonnet");
    assert!(hints.is_some());
    assert!(hints.unwrap().contains("Claude"));
}

#[test]
fn model_hints_openai() {
    let hints = model_hints("gpt-4o");
    assert!(hints.is_some());
    assert!(hints.unwrap().contains("OpenAI"));
}

#[test]
fn model_hints_openai_reasoning() {
    let hints = model_hints("o3-mini");
    assert!(hints.is_some());
    assert!(hints.unwrap().contains("reasoning"));
}

#[test]
fn model_hints_gemini() {
    let hints = model_hints("gemini-2.0-flash");
    assert!(hints.is_some());
    assert!(hints.unwrap().contains("Gemini"));
}

#[test]
fn model_hints_deepseek() {
    let hints = model_hints("deepseek-coder-v2");
    assert!(hints.is_some());
    assert!(hints.unwrap().contains("DeepSeek"));
}

#[test]
fn model_hints_unknown_returns_none() {
    assert!(model_hints("some-random-model").is_none());
}

#[test]
fn environment_block_format() {
    let block = build_environment_block("test-model", Path::new("/workspace"), true);
    assert!(block.starts_with("<environment>"));
    assert!(block.ends_with("</environment>"));
    assert!(block.contains("Model: test-model"));
    assert!(block.contains("Git repository: yes"));
    assert!(block.contains("Today's date:"));
    assert!(block.contains("Shell:"));
}

#[test]
fn tool_summary_format() {
    let summary = build_tool_summary(&mock_tools());
    assert!(summary.contains("<tools>"));
    assert!(summary.contains("**read**: Read file contents"));
    assert!(summary.contains("**bash**: Execute a shell command"));
    assert!(summary.contains("</tools>"));
}

#[test]
fn tool_summary_empty() {
    assert!(build_tool_summary(&[]).is_empty());
}

#[test]
fn format_today_has_expected_shape() {
    let today = format_today();
    assert!(today.len() >= 15, "date too short: {today}");
    assert!(today.contains('-'), "no dash: {today}");
    assert!(today.contains('('), "no paren: {today}");
}

#[test]
fn days_to_ymd_epoch() {
    let (y, m, d) = days_to_ymd(0);
    assert_eq!((y, m, d), (1970, 1, 1));
}

#[test]
fn days_to_ymd_known_date() {
    let (y, m, d) = days_to_ymd(19723);
    assert_eq!((y, m, d), (2024, 1, 1));
}

#[test]
fn is_git_repo_detects_git_dir() {
    assert!(!is_git_repo(Path::new("/tmp/nonexistent-git-test")));
}

#[test]
fn mode_section_plan_contains_read_only() {
    let section = mode_section("plan").expect("plan mode should have a section");
    assert!(section.contains("Plan mode"));
    assert!(section.contains("read-only"));
    assert!(section.contains("Build mode"));
}

#[test]
fn mode_section_build_contains_approval() {
    let section = mode_section("build").expect("build mode should have a section");
    assert!(section.contains("Build mode"));
    assert!(section.contains("approval"));
}

#[test]
fn mode_section_yolo_contains_auto_approved() {
    let section = mode_section("yolo").expect("yolo mode should have a section");
    assert!(section.contains("auto-approved"));
}

#[test]
fn mode_section_accept_edits_contains_file_operations() {
    let section = mode_section("accept_edits").expect("accept_edits mode should have a section");
    assert!(section.contains("File operations"));
    assert!(section.contains("auto-approved"));
}

#[test]
fn mode_section_unknown_returns_none() {
    assert!(mode_section("unknown").is_none());
}

#[test]
fn build_system_prompt_includes_mode_section() {
    let prompt = build_system_prompt(
        "gpt-4o",
        Path::new("/tmp/test"),
        false,
        &mock_tools(),
        &[],
        None,
        Some("plan"),
    );
    assert!(prompt.contains("Active Mode: Plan"));
    assert!(prompt.contains("read-only"));
}
