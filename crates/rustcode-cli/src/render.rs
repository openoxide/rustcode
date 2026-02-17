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

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use rustcode_core::event::{Event, EventPayload, EventScope};

    use super::{render_event, OutputFormat};

    fn sample_event() -> Event {
        Event {
            id: 42,
            timestamp: UNIX_EPOCH,
            scope: EventScope::Command,
            payload: EventPayload::OutputChunk {
                text: "hello".to_string(),
            },
        }
    }

    #[test]
    fn human_renderer_contract_is_stable() {
        let rendered = render_event(&sample_event(), OutputFormat::Human).expect("must render");
        assert_eq!(rendered, "[Command] OutputChunk { text: \"hello\" }");
    }

    #[test]
    fn json_renderer_contract_is_stable_and_parseable() {
        let rendered = render_event(&sample_event(), OutputFormat::Json).expect("must render");
        assert_eq!(
            rendered,
            "{\"id\":42,\"timestamp\":{\"secs_since_epoch\":0,\"nanos_since_epoch\":0},\"scope\":\"Command\",\"payload\":{\"type\":\"OutputChunk\",\"data\":{\"text\":\"hello\"}}}"
        );

        let reparsed: Event = serde_json::from_str(&rendered).expect("must parse");
        assert_eq!(reparsed, sample_event());
    }
}
