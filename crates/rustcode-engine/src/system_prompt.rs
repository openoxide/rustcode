// ── System prompt builder ─────────────────────────────────────────────
//
// Constructs the full system prompt for the rustcode agent, including:
//   • Base identity and behavior guidelines
//   • Editing constraints and tool usage policy
//   • Git & workspace hygiene rules
//   • Model-specific hints
//   • Available tools summary
//   • Environment context (working dir, platform, date, git status)
//   • Loaded instruction files

use std::path::Path;

use crate::instructions;

/// Base identity prompt for the rustcode agent.
const BASE_PROMPT: &str = "\
You are rustcode, a powerful, production-grade AI coding agent.

You are an interactive CLI tool that helps users with software engineering tasks. \
Use the instructions below and the tools available to you to assist the user.

## Editing Constraints
- Default to ASCII when editing or creating files. Only introduce non-ASCII or \
Unicode characters when the file already uses them.
- Only add comments when necessary to explain a non-obvious block of code.
- Prefer `apply_patch` for single-file edits. Use `write` for new files or \
auto-generated content. Use `bash` for scripting bulk changes (e.g., search-replace \
across the codebase).
- Always read a file before editing it. Never guess at file contents.

## Tool Usage Policy
- Prefer specialized tools over shell for file operations:
  - Use `read` to view files, `edit` to modify files, and `write` only when creating \
new files or replacing entire contents.
  - Use `glob` to find files by name and `grep` to search file contents.
  - Use `bash` for terminal operations (git, build, tests, running scripts).
- Run tool calls in parallel when neither call needs the other's output; otherwise \
run sequentially.
- Never use placeholder values for required tool arguments. If a value is unknown, \
read the relevant code first.

## Git & Workspace Hygiene
- You may be in a dirty git worktree.
  - NEVER revert existing changes you did not make unless explicitly requested.
  - If there are unrelated changes in files you need to modify, work around them.
  - If changes are in unrelated files, ignore them.
- Do not amend commits unless explicitly requested.
- NEVER use destructive commands like `git reset --hard` or `git checkout --` unless \
specifically requested by the user.

## Response Style
- Be concise and direct. Avoid unnecessary explanations or filler.
- Default: do the work without asking questions. Treat short tasks as sufficient \
direction; infer details by reading the codebase and following existing conventions.
- Only ask when you are truly blocked: the request is ambiguous in a way that \
materially changes the result, the action is destructive or irreversible, or you need \
a secret/credential that cannot be inferred.
- For code changes: lead with a quick explanation of what changed and why, then \
suggest natural next steps if any exist.
- Reference files with inline code using the pattern `path/to/file:line_number`.
- When uncertain, ask the user for clarification rather than guessing.
- Prefer reading existing code before making changes.
- Use the `bash` tool for compilation, tests, and verification.
- Respect the user's existing code style and conventions.
- When done, respond with a clear, final answer without tool calls.

## Planning & Tracking
- Do NOT use the `plan` tool unless the user explicitly asks you to plan, or the task \
genuinely requires coordinating 5+ distinct steps across multiple files. Most tasks — \
bug fixes, small features, refactors, single-file changes — should be done directly \
without planning. Bias heavily toward action over planning.
- Use `todowrite` only for large multi-file tasks where tracking progress is genuinely \
useful. Do not create todos for simple or moderate tasks.
- When you do use plans or todos, keep them updated with step statuses.";

/// Build the full system prompt.
///
/// Combines the base identity, persistent memory, model-specific hints, tool summary,
/// available skills, environment block, mode-specific behaviour, and loaded instruction files.
#[must_use]
pub fn build_system_prompt(
    model: &str,
    workspace_root: &Path,
    is_git_repo: bool,
    tools: &[rustcode_llm::ToolSpec],
    skills: &[rustcode_skills::SkillFile],
    memory_summary: Option<&str>,
    mode_hint: Option<&str>,
) -> String {
    let mut parts = Vec::with_capacity(8);

    // 1. Base identity + guidelines
    parts.push(BASE_PROMPT.to_string());

    // 2. Persistent memory (injected early so the model treats it as context)
    if let Some(mem) = memory_summary {
        let section = rustcode_memories::build_memory_section(mem);
        if !section.is_empty() {
            parts.push(section);
        }
    }

    // 3. Model-specific hints
    if let Some(hints) = model_hints(model) {
        parts.push(hints.to_string());
    }

    // 4. Available tools summary
    let tool_summary = build_tool_summary(tools);
    if !tool_summary.is_empty() {
        parts.push(tool_summary);
    }

    // 5. Available skills
    let skills_section = rustcode_skills::build_skills_section(skills);
    if !skills_section.is_empty() {
        parts.push(skills_section);
    }

    // 6. Environment block
    parts.push(build_environment_block(model, workspace_root, is_git_repo));

    // 7. Mode-specific behaviour
    if let Some(section) = mode_hint.and_then(mode_section) {
        parts.push(section.to_string());
    }

    // 8. Loaded instructions
    let instruction_files = instructions::load_instructions(workspace_root);
    let formatted = instructions::format_instructions(&instruction_files);
    if !formatted.is_empty() {
        parts.push(formatted);
    }

    parts.join("\n\n")
}

/// Short mode reminder injected before each user message in ongoing conversations.
///
/// Ensures the LLM picks up mode switches (e.g. Plan → Build) even when the
/// conversation history is long.
#[must_use]
pub fn mode_reminder(mode: &str) -> Option<&'static str> {
    match mode {
        "build" => Some(
            "[Mode switched to Build] You are now in Build mode. \
             All tools are available but require user approval. Proceed with implementation.",
        ),
        "accept_edits" => Some(
            "[Mode switched to Accept Edits] File operations are auto-approved. \
             Shell commands still require approval. Prefer file tools over bash.",
        ),
        "yolo" => Some(
            "[Mode switched to Yolo] All tools are auto-approved. \
             Work efficiently. Be careful with destructive operations.",
        ),
        "plan" => Some(
            "[Mode switched to Plan] You are in read-only Plan mode. \
             Do NOT call write/edit/bash/exec tools — they will be denied. \
             Only use read, glob, grep, list, codesearch, plan, todowrite, question. \
             When ready to implement, tell the user to switch to Build mode (Shift+Tab).",
        ),
        _ => None,
    }
}

/// Mode-specific system prompt section.
///
/// Returns behavioural instructions tailored to the current approval mode.
fn mode_section(mode: &str) -> Option<&'static str> {
    match mode {
        "build" => Some(
            "## Active Mode: Build\n\
             You are in **Build mode** — the standard working mode.\n\
             - All tool calls (file writes, edits, commands) require user approval before execution.\n\
             - Proceed normally: read code, analyze, then make changes. The user will approve or \
             deny each tool call interactively.\n\
             - Do not ask whether to proceed — just call the tools and the user will decide.",
        ),
        "accept_edits" => Some(
            "## Active Mode: Accept Edits\n\
             You are in **Accept Edits mode**.\n\
             - File operations (read, write, edit, glob, grep, apply_patch, multiedit) are \
             **auto-approved** — you can freely read and modify files without waiting.\n\
             - Shell commands (bash, exec, pty_exec) still require user approval.\n\
             - Prefer file-based tools over shell commands when possible. For example, use \
             `write` or `apply_patch` instead of `bash` with sed/echo.\n\
             - Do not ask whether to proceed with file changes — they are approved automatically.",
        ),
        "yolo" => Some(
            "## Active Mode: Yolo\n\
             You are in **Yolo mode** — all tools are auto-approved.\n\
             - Every tool call (file writes, edits, shell commands) is approved automatically.\n\
             - Work efficiently: chain tool calls, run tests, make changes without hesitation.\n\
             - Be extra careful with destructive operations (deleting files, force-pushing, \
             dropping data) — there is no approval gate, so double-check before executing \
             anything irreversible.\n\
             - Do not ask whether to proceed — just do the work.",
        ),
        "plan" => Some(
            "## Active Mode: Plan\n\
             You are in **Plan mode** — read-only research and analysis.\n\
             - You may ONLY use read-only tools: `read`, `glob`, `grep`, `list`, `codesearch`, \
             `plan`, `todowrite`, `question`.\n\
             - ALL write/edit/exec tools will be **automatically denied**. Do NOT call `write`, \
             `apply_patch`, `multiedit`, `bash`, `exec`, or `pty_exec` — they will fail.\n\
             - Focus on understanding the codebase: read files, search for patterns, analyze \
             architecture, gather context, and formulate a plan.\n\
             - Present your findings and proposed plan to the user.\n\
             - When you have finished your analysis and are ready to implement changes, \
             tell the user: \"I've completed my analysis. Switch to Build mode (Shift+Tab) \
             to start implementing the changes.\"\n\
             - ALWAYS end your final response with a suggestion to switch to Build mode \
             when there is work to be done.",
        ),
        _ => None,
    }
}

/// Model-specific behavior hints.
fn model_hints(model: &str) -> Option<&'static str> {
    let model_lower = model.to_lowercase();
    if model_lower.contains("claude") {
        Some(
            "## Model Notes\n\
             You are running on a Claude model. Use extended thinking for complex, \
             multi-step reasoning tasks. Structure tool calls with explicit IDs. \
             Be aware that Claude excels at careful, step-by-step analysis.",
        )
    } else if model_lower.contains("gpt")
        || model_lower.contains("o1")
        || model_lower.contains("o3")
        || model_lower.contains("o4")
    {
        Some(
            "## Model Notes\n\
             You are running on an OpenAI model. Be precise with function calling \
             syntax. Use structured outputs when available. For reasoning models \
             (o1/o3/o4), leverage built-in chain-of-thought capabilities.",
        )
    } else if model_lower.contains("gemini") {
        Some(
            "## Model Notes\n\
             You are running on a Google Gemini model. Leverage grounding \
             capabilities when performing web searches. Be thorough with \
             multi-modal inputs when available.",
        )
    } else if model_lower.contains("deepseek") {
        Some(
            "## Model Notes\n\
             You are running on a DeepSeek model. Focus on code-centric responses \
             and be thorough with technical details. DeepSeek models excel at \
             code understanding and generation.",
        )
    } else {
        None
    }
}

/// Build a concise summary of available tools for the system prompt.
///
/// Produces a `<tools>` block listing each tool with its description.
/// This gives the LLM awareness of its capabilities beyond the API tool definitions.
#[must_use]
pub fn build_tool_summary(tools: &[rustcode_llm::ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Available Tools\n\n<tools>\n");
    for tool in tools {
        out.push_str(&format!("- **{}**: {}\n", tool.name, tool.description));
    }
    out.push_str("</tools>");
    out
}

/// Build the environment context block.
fn build_environment_block(model: &str, workspace_root: &Path, is_git_repo: bool) -> String {
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unknown"
    };

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let today = format_today();

    format!(
        "<environment>\n\
         Model: {model}\n\
         Working directory: {workspace}\n\
         Git repository: {git}\n\
         Platform: {platform}\n\
         Shell: {shell}\n\
         Today's date: {today}\n\
         </environment>",
        workspace = workspace_root.display(),
        git = if is_git_repo { "yes" } else { "no" },
    )
}

/// Format today's date as "YYYY-MM-DD (Day)" using `std::time`.
fn format_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Convert unix timestamp to date components
    let days = (secs / 86400) as i64;
    let (year, month, day) = days_to_ymd(days);

    let weekday = match (days % 7 + 4) % 7 {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    };

    format!("{year}-{month:02}-{day:02} ({weekday})")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Check if a directory is a git repository.
#[must_use]
pub fn is_git_repo(workspace_root: &Path) -> bool {
    workspace_root.join(".git").exists()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        // Should match pattern like "2026-02-19 (Wed)"
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
        // 2024-01-01 = day 19723
        let (y, m, d) = days_to_ymd(19723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }

    #[test]
    fn is_git_repo_detects_git_dir() {
        // /tmp is unlikely to be a git repo
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
        let section =
            mode_section("accept_edits").expect("accept_edits mode should have a section");
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
}
