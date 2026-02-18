//! Context compaction for the agent loop.
//!
//! When the context window approaches its limit, this module:
//! 1. Prunes old tool call outputs (keeping recent ones)
//! 2. Sends the conversation to the LLM for summarization
//! 3. Replaces the message history with: system prompt + summary + recent messages

use rustcode_llm::{ChatMessage, ChatRequest, ChatRole, RequestInitiator};
use serde_json::Value;

use super::{CommandContext, Engine, ExecutionError};

/// Number of recent user turns to preserve during compaction.
/// Messages within this many turns from the end are never pruned.
const PROTECT_RECENT_TURNS: usize = 2;

/// Default prompt sent to the LLM to generate a conversation summary.
const SUMMARIZATION_PROMPT: &str = "\
Provide a detailed summary of our conversation so far.

Focus on information needed to continue working effectively:

## Goal
What is the user trying to accomplish?

## Key Decisions
Important technical decisions and constraints.

## Work Done
What has been completed, what is in progress.

## Relevant Files
Files that have been read, edited, or created.

## Next Steps
What needs to happen next.

Be comprehensive but concise. This summary will replace the full conversation history.";

/// Prune old tool call outputs from the message history, keeping recent ones.
///
/// Walks backwards through messages. Tool results beyond `protect_recent_turns`
/// user turns ago are replaced with a short placeholder to reduce token count.
///
/// Returns the number of tool outputs pruned.
pub fn prune_tool_outputs(messages: &mut [ChatMessage], protect_recent_turns: usize) -> usize {
    let mut user_turns_from_end = 0;
    let mut pruned = 0;

    // Walk backwards to count user turns
    for msg in messages.iter_mut().rev() {
        if msg.role == ChatRole::User {
            user_turns_from_end += 1;
        }

        // Only prune tool results beyond the protected window
        if user_turns_from_end > protect_recent_turns && msg.role == ChatRole::Tool {
            let content_str = msg.content.as_str().unwrap_or("");
            // Only prune if the content is significantly sized (>200 chars)
            if content_str.len() > 200 {
                msg.content =
                    Value::String("[tool output pruned for context management]".to_string());
                pruned += 1;
            }
        }
    }

    pruned
}

/// Build the messages for a compaction summarization request.
///
/// Takes the current conversation and asks the LLM to summarize it.
fn build_compaction_request(messages: &[ChatMessage], model: &str) -> ChatRequest {
    // Include the full conversation as context, then ask for summary
    let mut summary_messages = messages.to_vec();
    summary_messages.push(ChatMessage {
        role: ChatRole::User,
        content: Value::String(SUMMARIZATION_PROMPT.to_string()),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    });

    ChatRequest {
        model: model.to_string(),
        messages: summary_messages,
        tools: Vec::new(), // No tools for compaction
        initiator: RequestInitiator::Agent,
    }
}

/// Build a new message history from a compaction summary.
///
/// Returns: [system_prompt, summary_as_assistant_msg, recent_user_messages...]
fn build_compacted_history(
    system_prompt: &str,
    summary_text: &str,
    recent_messages: &[ChatMessage],
) -> Vec<ChatMessage> {
    let mut compacted = Vec::new();

    // 1. System prompt
    compacted.push(ChatMessage {
        role: ChatRole::System,
        content: Value::String(system_prompt.to_string()),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    });

    // 2. Summary as an assistant message
    let summary_header = format!(
        "[Context compacted — previous conversation summarized below]\n\n{summary_text}"
    );
    compacted.push(ChatMessage {
        role: ChatRole::Assistant,
        content: Value::String(summary_header),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    });

    // 3. Recent messages (preserve the tail of the conversation)
    compacted.extend_from_slice(recent_messages);

    compacted
}

/// Extract recent messages to preserve during compaction.
///
/// Keeps the last N user turns and everything after them.
fn extract_recent_messages(messages: &[ChatMessage], keep_turns: usize) -> &[ChatMessage] {
    if keep_turns == 0 || messages.is_empty() {
        return &[];
    }

    let mut user_turns_seen = 0;
    let mut split_at = messages.len();

    for (idx, msg) in messages.iter().enumerate().rev() {
        if msg.role == ChatRole::User {
            user_turns_seen += 1;
            if user_turns_seen >= keep_turns {
                split_at = idx;
                break;
            }
        }
    }

    &messages[split_at..]
}

/// Extract the system prompt from the message history.
fn extract_system_prompt(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .find(|m| m.role == ChatRole::System)
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_string()
}

impl Engine {
    /// Run context compaction: prune old outputs, summarize via LLM,
    /// and replace message history with summary + recent messages.
    ///
    /// Returns `Ok(())` if compaction succeeded, updating `messages` in place.
    /// Returns `Err(...)` if the summarization LLM call fails (original
    /// messages are still pruned but not replaced).
    pub(crate) async fn compact_context(
        &self,
        messages: &mut Vec<ChatMessage>,
        context: &CommandContext,
    ) -> Result<(), ExecutionError> {
        tracing::info!(
            "context compaction triggered (messages={})",
            messages.len()
        );

        // Step 1: Prune old tool outputs
        let pruned = prune_tool_outputs(messages, PROTECT_RECENT_TURNS);
        tracing::debug!("pruned {} tool outputs", pruned);

        // Step 2: Build and send summarization request
        let request = build_compaction_request(messages, &context.config.model);
        let summary_response = self.llm.chat(request).await.map_err(|err| {
            ExecutionError::Executor(format!("compaction summarization failed: {err}"))
        })?;

        if summary_response.text.is_empty() {
            tracing::warn!("compaction summary was empty, keeping pruned messages");
            return Ok(());
        }

        // Step 3: Build compacted history
        let system_prompt = extract_system_prompt(messages);
        let recent = extract_recent_messages(messages, PROTECT_RECENT_TURNS);
        let compacted =
            build_compacted_history(&system_prompt, &summary_response.text, recent);

        tracing::info!(
            "compacted context: {} messages → {} messages",
            messages.len(),
            compacted.len()
        );

        *messages = compacted;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            content: Value::String(text.to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: Value::String(text.to_string()),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    fn tool_msg(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Tool,
            content: Value::String(content.to_string()),
            tool_call_id: Some(id.to_string()),
            tool_name: Some("read".to_string()),
            tool_calls: Vec::new(),
        }
    }

    #[test]
    fn prune_leaves_recent_tools() {
        let mut msgs = vec![
            user_msg("first"),
            assistant_msg("ok"),
            tool_msg("1", &"x".repeat(300)),
            user_msg("second"),
            assistant_msg("ok"),
            tool_msg("2", &"y".repeat(300)),
            user_msg("third"),
            assistant_msg("ok"),
            tool_msg("3", &"z".repeat(300)),
            user_msg("fourth"),
            assistant_msg("ok"),
        ];
        let pruned = prune_tool_outputs(&mut msgs, 2);
        // Tool "1" is at user_turns=3 (> 2), so it's pruned
        // Tool "2" is at user_turns=2 (== 2, not >), so it stays
        // Tool "3" is at user_turns=1, within the protected window
        assert_eq!(pruned, 1);
        assert!(msgs[2].content.as_str().unwrap().contains("pruned"));
        assert!(!msgs[5].content.as_str().unwrap().contains("pruned"));
        assert!(!msgs[8].content.as_str().unwrap().contains("pruned"));
    }

    #[test]
    fn prune_skips_small_outputs() {
        let mut msgs = vec![
            user_msg("first"),
            tool_msg("1", "small"),
            user_msg("second"),
            user_msg("third"),
            user_msg("fourth"),
        ];
        let pruned = prune_tool_outputs(&mut msgs, 2);
        assert_eq!(pruned, 0); // "small" is under 200 chars
    }

    #[test]
    fn extract_recent_messages_works() {
        let msgs = vec![
            user_msg("first"),
            assistant_msg("a"),
            user_msg("second"),
            assistant_msg("b"),
            user_msg("third"),
            assistant_msg("c"),
        ];
        let recent = extract_recent_messages(&msgs, 2);
        assert_eq!(recent.len(), 4); // second + a + third + c
        assert_eq!(recent[0].content.as_str().unwrap(), "second");
    }

    #[test]
    fn build_compacted_history_structure() {
        let recent = vec![user_msg("latest question"), assistant_msg("answer")];
        let compacted = build_compacted_history("You are helpful.", "Summary of work", &recent);
        assert_eq!(compacted.len(), 4);
        assert_eq!(compacted[0].role, ChatRole::System);
        assert_eq!(compacted[1].role, ChatRole::Assistant);
        assert!(compacted[1]
            .content
            .as_str()
            .unwrap()
            .contains("Summary of work"));
        assert_eq!(compacted[2].role, ChatRole::User);
        assert_eq!(compacted[3].role, ChatRole::Assistant);
    }

    #[test]
    fn extract_system_prompt_finds_it() {
        let msgs = vec![
            ChatMessage {
                role: ChatRole::System,
                content: Value::String("system prompt".to_string()),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            },
            user_msg("hello"),
        ];
        assert_eq!(extract_system_prompt(&msgs), "system prompt");
    }

    #[test]
    fn extract_system_prompt_missing() {
        let msgs = vec![user_msg("hello")];
        assert_eq!(extract_system_prompt(&msgs), "");
    }
}
