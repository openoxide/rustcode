use anyhow::Result;

use rustcode_core::event::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

impl OutputFormat {
    pub fn from_json_flag(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Human
        }
    }
}

pub fn render_event(event: &Event, format: OutputFormat) -> Result<String> {
    let rendered = match format {
        OutputFormat::Human => format!("[{:?}] {:?}", event.scope, event.payload),
        OutputFormat::Json => serde_json::to_string(event)?,
    };

    Ok(rendered)
}
