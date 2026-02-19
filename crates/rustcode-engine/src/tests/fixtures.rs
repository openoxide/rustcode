use super::*;

pub(super) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) struct CancelledProcess;
pub(super) struct StubProcess {
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) code: i32,
}
pub(super) struct CountingApprover {
    pub(super) approved: bool,
    pub(super) calls: AtomicUsize,
}
pub(super) struct DummyFs;
pub(super) struct StreamingLlmClient;
pub(super) struct AgentFs {
    pub(super) root: PathBuf,
}
pub(super) struct SearchFs {
    pub(super) root: PathBuf,
}
pub(super) struct ScriptedAgentLlm {
    pub(super) step: Mutex<usize>,
}

#[async_trait]
impl FileSystemPort for DummyFs {
    async fn read_to_string(&self, _path: &Path) -> Result<String, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn read_to_string_limited(
        &self,
        _path: &Path,
        _max_bytes: usize,
    ) -> Result<String, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn exists(&self, _path: &Path) -> Result<bool, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn list_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn metadata(&self, _path: &Path) -> Result<rustcode_io::FsMetadata, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn walk_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Err(IoError::Io("not used".to_string()))
    }
}

#[async_trait]
impl FileSystemPort for AgentFs {
    async fn read_to_string(&self, _path: &Path) -> Result<String, IoError> {
        Ok("agent-read-ok".to_string())
    }

    async fn read_to_string_limited(
        &self,
        _path: &Path,
        _max_bytes: usize,
    ) -> Result<String, IoError> {
        Ok("agent-read-ok".to_string())
    }

    async fn exists(&self, _path: &Path) -> Result<bool, IoError> {
        Ok(true)
    }

    async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
        Ok(())
    }

    async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
        Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
    }

    async fn list_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
    }

    async fn metadata(&self, path: &Path) -> Result<rustcode_io::FsMetadata, IoError> {
        let is_dir = path.ends_with("dir");
        Ok(rustcode_io::FsMetadata {
            is_dir,
            is_file: !is_dir,
            len: 0,
        })
    }

    async fn walk_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Ok(vec![self.root.join("a.txt"), self.root.join("dir")])
    }
}

#[async_trait]
impl FileSystemPort for SearchFs {
    async fn read_to_string(&self, path: &Path) -> Result<String, IoError> {
        self.read_to_string_limited(path, usize::MAX).await
    }

    async fn read_to_string_limited(
        &self,
        path: &Path,
        _max_bytes: usize,
    ) -> Result<String, IoError> {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("<unknown>");
        let content = match name {
            "a.txt" => "hello\nneedle here\n".to_string(),
            "b.md" => "needle too\n".to_string(),
            _ => "".to_string(),
        };
        Ok(content)
    }

    async fn exists(&self, _path: &Path) -> Result<bool, IoError> {
        Ok(true)
    }

    async fn write_string(&self, _path: &Path, _contents: &str) -> Result<(), IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn list_dir(&self, _path: &Path) -> Result<Vec<PathBuf>, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn list_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Err(IoError::Io("not used".to_string()))
    }

    async fn metadata(&self, path: &Path) -> Result<rustcode_io::FsMetadata, IoError> {
        let is_dir = path.ends_with("dir");
        Ok(rustcode_io::FsMetadata {
            is_dir,
            is_file: !is_dir,
            len: 0,
        })
    }

    async fn walk_dir_limited(
        &self,
        _path: &Path,
        _max_entries: usize,
    ) -> Result<Vec<PathBuf>, IoError> {
        Ok(vec![
            self.root.join("a.txt"),
            self.root.join("b.md"),
            self.root.join("dir"),
        ])
    }
}

#[async_trait]
impl ProcessPort for CancelledProcess {
    async fn run(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        Err(IoError::Cancelled)
    }

    async fn run_capture(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        Err(IoError::Cancelled)
    }

    async fn run_pty(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<rustcode_io::PtyOutput, IoError> {
        Err(IoError::Cancelled)
    }
}

#[async_trait]
impl ProcessPort for StubProcess {
    async fn run(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        if self.code == 0 {
            Ok(ProcessOutput {
                code: 0,
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        } else {
            Err(IoError::Exit {
                code: self.code,
                stderr: self.stderr.clone(),
            })
        }
    }

    async fn run_capture(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, IoError> {
        Ok(ProcessOutput {
            code: self.code,
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
        })
    }

    async fn run_pty(
        &self,
        _program: &str,
        _args: &[String],
        _cwd: &Path,
        _cancellation: CancellationToken,
    ) -> Result<rustcode_io::PtyOutput, IoError> {
        Ok(rustcode_io::PtyOutput {
            output: self.stdout.clone(),
            exit_code: self.code,
        })
    }
}

#[async_trait]
impl ToolApprover for CountingApprover {
    async fn approve(&self, _request: ToolApprovalRequest) -> Result<bool, ExecutionError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.approved)
    }
}

#[async_trait]
impl LlmClient for StreamingLlmClient {
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, rustcode_llm::LlmError> {
        Ok(LlmResponse {
            text: "hello world".to_string(),
            chunks: vec!["hello".to_string(), " world".to_string()],
        })
    }
}

#[async_trait]
impl LlmClient for ScriptedAgentLlm {
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, rustcode_llm::LlmError> {
        Ok(LlmResponse {
            text: "unused".to_string(),
            chunks: Vec::new(),
        })
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, rustcode_llm::LlmError> {
        let mut step = self.step.lock().await;
        match *step {
            0 => {
                *step = 1;
                assert!(
                    !request.tools.is_empty(),
                    "agent request must include tools"
                );
                Ok(ChatResponse {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "list".to_string(),
                        arguments: r#"{"path":"."}"#.to_string(),
                    }],
                    usage: None,
                })
            }
            _ => {
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|msg| msg.tool_call_id.as_deref() == Some("call_1")),
                    "expected tool result message for call_1"
                );
                Ok(ChatResponse {
                    text: "done".to_string(),
                    tool_calls: Vec::new(),
                    usage: None,
                })
            }
        }
    }
}

#[derive(Default)]
pub(super) struct CollectingPublisher {
    pub(super) events: Mutex<Vec<Event>>,
}

pub(super) struct CountingPlugin {
    pub(super) seen: Arc<Mutex<usize>>,
}

#[async_trait]
impl Plugin for CountingPlugin {
    fn name(&self) -> &'static str {
        "counting"
    }

    async fn on_event(&self, _event: &Event, _ctx: &CommandContext) -> Result<(), PluginError> {
        let mut seen = self.seen.lock().await;
        *seen += 1;
        Ok(())
    }
}

#[async_trait]
impl EventPublisher for CollectingPublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        self.events.lock().await.push(event);
        Ok(())
    }
}
