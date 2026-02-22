use async_trait::async_trait;

use rustcode_core::{ExecutionError, TranscriptRecorder};
use rustcode_core::session::StoredMessage;

use crate::{FileTranscriptRecorder, SessionStore};

impl FileTranscriptRecorder {
    #[must_use]
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl TranscriptRecorder for FileTranscriptRecorder {
    async fn append_message(
        &self,
        session_id: &str,
        message: StoredMessage,
    ) -> Result<(), ExecutionError> {
        let store = self.store.clone();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || store.append_message(&session_id, &message))
            .await
            .map_err(|err| {
                ExecutionError::Executor(format!("transcript recorder join error: {err}"))
            })?
            .map_err(|err| ExecutionError::Executor(err.to_string()))
    }
}
