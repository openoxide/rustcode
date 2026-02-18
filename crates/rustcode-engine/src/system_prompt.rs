// ── System prompt builder ─────────────────────────────────────────────
//
// Constructs the full system prompt for the rustcode agent, including:
//   • Base identity and behavior guidelines
//   • Model-specific hints
//   • Environment context (working dir, platform, date, git status)
//   • Loaded instruction files

use std::path::Path;

use crate::instructions;

/// Base identity prompt for the rustcode agent.
const BASE_PROMPT: &str = "\
You are rustcode, a powerful, production-grade AI coding assistant.
You operate as a command-line agent that can read, create, and edit files, \
execute shell commands, and search codebases to help users accomplish their goals.

## Core Workflow
1. **Understand** — read the user's request carefully and gather context.
2. **Plan** — break complex tasks into steps before acting.
3. **Execute** — use tools methodically: list → read → edit. Only modify files when required.
4. **Verify** — after making changes, confirm correctness (run tests, compile, etc.).

## Guidelines
- Be concise and direct. Avoid unnecessary explanations.
- When uncertain, ask the user for clarification rather than guessing.
- Prefer reading existing code before making changes.
- Use the `bash` tool for compilation, tests, and verification.
- Respect the user's existing code style and conventions.
- When done, respond with a clear, final answer without tool calls.";

/// Build the full system prompt.
///
/// Combines the base identity, model-specific hints, environment block,
/// and any loaded instruction files into a single string.
pub fn build_system_prompt(
    model: &str,
    workspace_root: &Path,
    is_git_repo: bool,
) -> String {
    let mut parts = Vec::with_capacity(4);

    // 1. Base identity
    parts.push(BASE_PROMPT.to_string());

    // 2. Model-specific hints
    if let Some(hints) = model_hints(model) {
        parts.push(hints.to_string());
    }

    // 3. Environment block
    parts.push(build_environment_block(model, workspace_root, is_git_repo));

    // 4. Loaded instructions
    let instruction_files = instructions::load_instructions(workspace_root);
    let formatted = instructions::format_instructions(&instruction_files);
    if !formatted.is_empty() {
        parts.push(formatted);
    }

    parts.join("\n\n")
}

/// Model-specific behavior hints.
fn model_hints(model: &str) -> Option<&'static str> {
    let model_lower = model.to_lowercase();
    if model_lower.contains("claude") {
        Some("## Model Notes\n\
              You are running on a Claude model. Use extended thinking for complex tasks. \
              Structure tool calls clearly with explicit IDs.")
    } else if model_lower.contains("gpt") || model_lower.contains("o1") || model_lower.contains("o3") {
        Some("## Model Notes\n\
              You are running on an OpenAI model. Be precise with function calling syntax. \
              Use structured outputs when available.")
    } else if model_lower.contains("gemini") {
        Some("## Model Notes\n\
              You are running on a Google Gemini model. Leverage grounding capabilities \
              when performing web searches.")
    } else if model_lower.contains("deepseek") {
        Some("## Model Notes\n\
              You are running on a DeepSeek model. Focus on code-centric responses \
              and be thorough with technical details.")
    } else {
        None
    }
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

    format!(
        "<environment>\n\
         Model: {model}\n\
         Working directory: {workspace}\n\
         Git repository: {git}\n\
         Platform: {platform}\n\
         </environment>",
        workspace = workspace_root.display(),
        git = if is_git_repo { "yes" } else { "no" },
    )
}

/// Check if a directory is a git repository.
pub fn is_git_repo(workspace_root: &Path) -> bool {
    workspace_root.join(".git").exists()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_system_prompt_contains_identity() {
        let prompt = build_system_prompt("gpt-4o", Path::new("/tmp/test"), false);
        assert!(prompt.contains("You are rustcode"));
        assert!(prompt.contains("production-grade"));
    }

    #[test]
    fn build_system_prompt_includes_environment() {
        let prompt = build_system_prompt("claude-3.5-sonnet", Path::new("/project"), true);
        assert!(prompt.contains("<environment>"));
        assert!(prompt.contains("claude-3.5-sonnet"));
        assert!(prompt.contains("/project"));
        assert!(prompt.contains("Git repository: yes"));
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
    }

    #[test]
    fn is_git_repo_detects_git_dir() {
        // /tmp is unlikely to be a git repo
        assert!(!is_git_repo(Path::new("/tmp/nonexistent-git-test")));
    }
}
