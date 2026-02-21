use crate::types::LlmError;
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

pub(crate) enum StreamedProviderBody {
    Body(String),
    SseEvents(Vec<String>),
}

pub(crate) async fn read_sse_or_body(
    response: reqwest::Response,
    idle_timeout: Duration,
) -> Result<StreamedProviderBody, LlmError> {
    let mut stream = response.bytes_stream();
    let mut raw_body = String::new();
    let mut parse_buffer = String::new();
    let mut events = Vec::new();
    let mut timed_out_with_partial_events = false;

    loop {
        let Ok(chunk) = timeout(idle_timeout, stream.next()).await else {
            // Fallback: if we've already received complete SSE events, keep
            // the partial response instead of failing the whole run.
            if !events.is_empty() {
                timed_out_with_partial_events = true;
                break;
            }
            return Err(LlmError::Transport(
                "timed out waiting for provider response chunk".to_string(),
            ));
        };

        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|err| LlmError::Transport(err.to_string()))?;
        let text = String::from_utf8_lossy(&chunk);
        raw_body.push_str(&text);
        parse_buffer.push_str(&text);

        while let Some(block) = take_next_sse_block(&mut parse_buffer) {
            if let Some(data) = parse_sse_data_block(&block) {
                events.push(data);
            }
        }
    }

    if events.is_empty() {
        return Ok(StreamedProviderBody::Body(raw_body));
    }

    // Only parse trailing data when the stream ended cleanly. On timeout,
    // trailing buffer is likely an incomplete SSE event fragment.
    if !timed_out_with_partial_events {
        if let Some(trailing) = parse_sse_data_block(parse_buffer.trim()) {
            events.push(trailing);
        }
    }
    Ok(StreamedProviderBody::SseEvents(events))
}

pub(crate) fn take_next_sse_block(buffer: &mut String) -> Option<String> {
    let mut separator: Option<(usize, usize)> = None;

    if let Some(index) = buffer.find("\n\n") {
        separator = Some((index, 2));
    }
    if let Some(index) = buffer.find("\r\n\r\n") {
        match separator {
            Some((current, _)) if current <= index => {}
            _ => separator = Some((index, 4)),
        }
    }

    let (index, length) = separator?;
    let block = buffer[..index].to_string();
    buffer.drain(..index + length);
    Some(block)
}

pub(crate) fn parse_sse_data_block(block: &str) -> Option<String> {
    if block.is_empty() {
        return None;
    }

    let mut data_lines = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

pub(crate) fn extract_stream_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    if let Some(message) = error.as_str() {
        return Some(message.to_string());
    }
    Some(error.to_string())
}
