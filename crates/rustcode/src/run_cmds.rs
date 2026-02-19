use crate::render::{render_event, OutputFormat};
use crate::utils::{write_stdout_line, write_stdout_raw};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::ACCEPT;
use rustcode_core::event::{Event, EventPayload, EventScope};
use rustcode_core::ports::{CommandExecutor, EventPublisher};
use std::sync::Arc;

pub async fn run_attached(
    url: &str,
    prompt: Option<&str>,
    session: Option<&str>,
    output_format: OutputFormat,
    event_debug: bool,
) -> Result<()> {
    let client = reqwest::Client::new();
    let target = format!("{url}/v1/run");
    let mut payload = serde_json::Map::new();
    if let Some(p) = prompt {
        payload.insert(
            "prompt".to_string(),
            serde_json::Value::String(p.to_string()),
        );
    }
    if let Some(s) = session {
        payload.insert(
            "session".to_string(),
            serde_json::Value::String(s.to_string()),
        );
    }

    let response = client
        .post(&target)
        .header(ACCEPT, "text/event-stream")
        .json(&payload)
        .send()
        .await
        .context("failed to connect to server")?;
    if !response.status().is_success() {
        anyhow::bail!("server returned error: {}", response.status());
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut streamed_text_open = false;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read from stream")?;
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        while let Some(block) = pop_sse_block(&mut buffer) {
            if let Some(data) = extract_sse_data(&block) {
                let event: Event =
                    serde_json::from_str(&data).context("failed to parse event json")?;
                if matches!(output_format, OutputFormat::Human) && !event_debug {
                    match &event.payload {
                        EventPayload::OutputChunk { text } => {
                            write_stdout_raw(text)?;
                            streamed_text_open = true;
                            continue;
                        }
                        EventPayload::Completed => {
                            if streamed_text_open {
                                write_stdout_raw("\n")?;
                                streamed_text_open = false;
                            }
                            continue;
                        }
                        _ => {
                            if streamed_text_open {
                                write_stdout_raw("\n")?;
                                streamed_text_open = false;
                            }
                        }
                    }
                }
                let rendered = render_event(&event, output_format)?;
                write_stdout_line(&rendered)?;
            }
        }
    }

    if streamed_text_open {
        let _ = write_stdout_raw("\n");
    }

    Ok(())
}

fn pop_sse_block(buffer: &mut String) -> Option<String> {
    if let Some(pos) = buffer.find("\n\n") {
        let block = buffer.drain(..pos + 2).collect();
        return Some(block);
    }
    None
}

fn extract_sse_data(block: &str) -> Option<String> {
    for line in block.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            return Some(data.to_string());
        }
    }
    None
}

pub struct RemoteRunExecutor {
    url: String,
}

impl RemoteRunExecutor {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

#[async_trait]
impl CommandExecutor for RemoteRunExecutor {
    async fn execute(
        &self,
        command: rustcode_core::command::Command,
        _context: rustcode_core::context::CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), rustcode_core::error::ExecutionError> {
        let prompt = match command {
            rustcode_core::command::Command::Run { prompt } => Some(prompt),
            _ => None,
        };

        if let Err(err) = run_attached(
            &self.url,
            prompt.as_deref(),
            None,
            OutputFormat::Human,
            false,
        )
        .await
        {
            let _ = publisher
                .publish(Event::new(
                    0, // Dummy ID
                    EventScope::Command,
                    EventPayload::Failure {
                        message: err.to_string(),
                    },
                ))
                .await;
        }

        Ok(())
    }
}
