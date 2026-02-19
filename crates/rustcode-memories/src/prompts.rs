/// Preamble injected at the start of extraction prompts.
pub const EXTRACTION_PREAMBLE: &str = "\
You are a memory extraction assistant. Extract the most important facts, preferences, \
patterns, and decisions from this conversation that would be useful to remember in \
future sessions with this user.

Focus on:
- User's coding language and style preferences
- Project-specific patterns or constraints
- Important decisions made during the session
- Recurring preferences or dislikes
- Key technical context about the project

Format: concise bullet points, one fact per line starting with `- `.
Do not include generic advice or common knowledge. Only extract user-specific information.
If nothing worth remembering was discussed, respond with exactly: NO_MEMORIES
";

/// Preamble injected at the start of consolidation prompts.
pub const CONSOLIDATION_PREAMBLE: &str = "\
You are a memory consolidation assistant. Synthesize the following bullet-point memories \
from multiple sessions into a single, concise, well-organized summary.

Guidelines:
- Merge duplicate or similar facts into single entries
- Group related facts under short thematic headings (e.g. ## Languages, ## Project Patterns)
- Resolve contradictions by keeping the most specific/recent information
- Keep the total summary concise (10-30 bullet points)

Return only the consolidated markdown summary with no preamble.
";

/// Build the full extraction prompt from a window of conversation messages.
pub fn extraction_prompt(messages: &[(String, String)]) -> String {
    let mut out = EXTRACTION_PREAMBLE.to_string();
    out.push_str("\n---\n\nConversation:\n\n");

    for (role, content) in messages {
        let display_role = match role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            _ => continue,
        };
        // Truncate very long messages to avoid token overflow
        let truncated = if content.len() > 2000 {
            format!("{}...[truncated]", &content[..2000])
        } else {
            content.clone()
        };
        out.push_str(&format!("{display_role}: {truncated}\n\n"));
    }

    out.push_str("---\n\nExtract memorable facts as bullet points:");
    out
}

/// Build the full consolidation prompt from collected raw memory contents.
pub fn consolidation_prompt(raw_memories: &[String]) -> String {
    let mut out = CONSOLIDATION_PREAMBLE.to_string();
    out.push_str("\n---\n\nRaw memories from sessions:\n\n");

    for (i, content) in raw_memories.iter().enumerate() {
        out.push_str(&format!("Session {}:\n{}\n\n", i + 1, content));
    }

    out.push_str("---\n\nConsolidated summary:");
    out
}

/// Build the `## Persistent Memory` block for injection into the system prompt.
///
/// Returns an empty string if `content` is empty or whitespace-only.
#[must_use]
pub fn build_memory_section(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!(
        "## Persistent Memory\n\nThe following facts were remembered from previous sessions:\n\n{trimmed}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extraction_includes_roles() {
        let msgs = vec![
            ("user".to_string(), "I prefer snake_case".to_string()),
            ("assistant".to_string(), "Got it.".to_string()),
            ("system".to_string(), "ignored".to_string()),
        ];
        let prompt = extraction_prompt(&msgs);
        assert!(prompt.contains("User: I prefer snake_case"));
        assert!(prompt.contains("Assistant: Got it."));
        assert!(!prompt.contains("ignored"));
    }

    #[test]
    fn consolidation_includes_all_sessions() {
        let raw = vec![
            "- uses Rust\n- prefers async".to_string(),
            "- avoids unwrap".to_string(),
        ];
        let prompt = consolidation_prompt(&raw);
        assert!(prompt.contains("Session 1"));
        assert!(prompt.contains("Session 2"));
        assert!(prompt.contains("uses Rust"));
    }

    #[test]
    fn memory_section_empty_when_no_content() {
        assert!(build_memory_section("").is_empty());
        assert!(build_memory_section("   ").is_empty());
    }

    #[test]
    fn memory_section_wraps_content() {
        let section = build_memory_section("- uses Rust");
        assert!(section.contains("Persistent Memory"));
        assert!(section.contains("uses Rust"));
    }
}
