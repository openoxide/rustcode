use super::{
    codex_default_instructions, codex_input_from_messages, codex_instructions_from_messages,
    codex_tool_spec_value, default_reasoning_effort, normalize_openai_model,
    parse_codex_chat_sse_events,
};
use crate::types::{ChatMessage, ChatRole, ToolCall, ToolSpec};
use serde_json::json;

#[test]
fn openai_codex_53_falls_back_to_52() {
    assert_eq!(
        normalize_openai_model("openai", false, "gpt-5.3-codex"),
        "gpt-5.2-codex"
    );
}

#[test]
fn openai_codex_endpoint_keeps_requested_model() {
    assert_eq!(
        normalize_openai_model("openai", true, "gpt-5.3-codex"),
        "gpt-5.3-codex"
    );
}

#[test]
fn non_openai_models_are_not_rewritten() {
    assert_eq!(
        normalize_openai_model("openrouter", false, "gpt-5.3-codex"),
        "gpt-5.3-codex"
    );
}

#[test]
fn openai_gpt5_defaults_to_high_effort() {
    assert_eq!(
        default_reasoning_effort("openai", "gpt-5.2-codex"),
        Some("high")
    );
}

#[test]
fn non_openai_has_no_default_reasoning_effort() {
    assert_eq!(
        default_reasoning_effort("openrouter", "gpt-5.2-codex"),
        None
    );
}

#[test]
fn codex_instructions_use_system_messages() {
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: json!("sys-1"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::<ToolCall>::new(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: json!("hello"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::<ToolCall>::new(),
        },
        ChatMessage {
            role: ChatRole::System,
            content: json!([{"type":"text","text":"sys-2"}]),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::<ToolCall>::new(),
        },
    ];

    assert_eq!(
        codex_instructions_from_messages(&messages),
        "sys-1\n\nsys-2".to_string()
    );
}

#[test]
fn codex_instructions_fallback_to_default() {
    let messages = vec![ChatMessage {
        role: ChatRole::User,
        content: json!("hello"),
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::<ToolCall>::new(),
    }];
    assert_eq!(
        codex_instructions_from_messages(&messages),
        codex_default_instructions().to_string()
    );
}

#[test]
fn codex_input_uses_responses_shape() {
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: json!("sys"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: json!("hello"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: ChatRole::Assistant,
            content: json!("calling tool"),
            tool_call_id: None,
            tool_name: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "echo".to_string(),
                arguments: r#"{"text":"hello"}"#.to_string(),
            }],
        },
        ChatMessage {
            role: ChatRole::Tool,
            content: json!({"ok": true}),
            tool_call_id: Some("call_1".to_string()),
            tool_name: Some("echo".to_string()),
            tool_calls: Vec::new(),
        },
    ];

    let input = codex_input_from_messages(&messages);
    assert_eq!(
        input,
        vec![
            json!({
                "role": "user",
                "content": [{"type":"input_text","text":"hello"}]
            }),
            json!({
                "role": "assistant",
                "content": [{"type":"output_text","text":"calling tool"}]
            }),
            json!({
                "type":"function_call",
                "call_id":"call_1",
                "name":"echo",
                "arguments":"{\"text\":\"hello\"}"
            }),
            json!({
                "type":"function_call_output",
                "call_id":"call_1",
                "output":"{\"ok\":true}"
            }),
        ]
    );
}

#[test]
fn codex_tool_spec_uses_responses_function_format() {
    let spec = ToolSpec {
        name: "echo".to_string(),
        description: "Echo text".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"]
        }),
    };
    let value = codex_tool_spec_value(&spec);
    assert_eq!(value.get("type"), Some(&json!("function")));
    assert_eq!(value.get("name"), Some(&json!("echo")));
    assert_eq!(value.get("description"), Some(&json!("Echo text")));
    assert!(value.get("function").is_none());
}

#[test]
fn codex_stream_events_parse_text_tools_reasoning_and_usage() {
    let events = vec![
        json!({"type":"response.output_text.delta","delta":"hi"}).to_string(),
        json!({"type":"response.reasoning_summary_text.delta","delta":"think"}).to_string(),
        json!({
            "type":"response.output_item.done",
            "item":{"type":"function_call","call_id":"call_1","name":"echo","arguments":"{\"text\":\"hello\"}"}
        })
        .to_string(),
        json!({
            "type":"response.completed",
            "response":{"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":2}}}
        })
        .to_string(),
    ];

    let response =
        parse_codex_chat_sse_events(events).expect("codex stream should parse successfully");
    assert_eq!(response.text, "hi");
    assert_eq!(response.reasoning.as_deref(), Some("think"));
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].id, "call_1");
    assert_eq!(response.tool_calls[0].name, "echo");
    assert_eq!(
        response.usage.expect("usage should exist").total,
        15,
        "usage should be parsed from response.completed"
    );
}
