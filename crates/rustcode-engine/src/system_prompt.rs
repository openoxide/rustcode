// -- System prompt builder ------------------------------------------------
//
// Constructs the full system prompt for the rustcode agent, including:
//   - Base identity and behavior guidelines
//   - Editing constraints and tool usage policy
//   - Git & workspace hygiene rules
//   - Model-specific hints
//   - Available tools summary
//   - Environment context (working dir, platform, date, git status)
//   - Loaded instruction files

use std::path::Path;

use crate::instructions;

mod environment;
mod prompt_sections;
#[cfg(test)]
mod tests;

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

    parts.push(prompt_sections::base_prompt().to_string());

    if let Some(mem) = memory_summary {
        let section = rustcode_memories::build_memory_section(mem);
        if !section.is_empty() {
            parts.push(section);
        }
    }

    if let Some(hints) = model_hints(model) {
        parts.push(hints.to_string());
    }

    let tool_summary = build_tool_summary(tools);
    if !tool_summary.is_empty() {
        parts.push(tool_summary);
    }

    let skills_section = rustcode_skills::build_skills_section(skills);
    if !skills_section.is_empty() {
        parts.push(skills_section);
    }

    parts.push(environment::build_environment_block(
        model,
        workspace_root,
        is_git_repo,
    ));

    if let Some(section) = mode_hint.and_then(mode_section) {
        parts.push(section.to_string());
    }

    let instruction_files = instructions::load_instructions(workspace_root);
    let formatted = instructions::format_instructions(&instruction_files);
    if !formatted.is_empty() {
        parts.push(formatted);
    }

    parts.join("\n\n")
}

/// Short mode reminder injected before each user message in ongoing conversations.
///
/// Ensures the LLM picks up mode switches (e.g. Plan -> Build) even when the
/// conversation history is long.
#[must_use]
pub fn mode_reminder(mode: &str) -> Option<&'static str> {
    prompt_sections::mode_reminder(mode)
}

/// Mode-specific system prompt section.
fn mode_section(mode: &str) -> Option<&'static str> {
    prompt_sections::mode_section(mode)
}

/// Model-specific behavior hints.
fn model_hints(model: &str) -> Option<&'static str> {
    prompt_sections::model_hints(model)
}

/// Build a concise summary of available tools for the system prompt.
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

/// Check if a directory is a git repository.
#[must_use]
pub fn is_git_repo(workspace_root: &Path) -> bool {
    environment::is_git_repo(workspace_root)
}

#[cfg(test)]
fn format_today() -> String {
    environment::format_today_for_test()
}

#[cfg(test)]
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    environment::days_to_ymd_for_test(days)
}
