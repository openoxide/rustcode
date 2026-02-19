use super::{
    debug, serve_http, Arc, AsyncWriteExt, CancellationToken, Command, CommandContext,
    CommandExecutor, Engine, EventPublisher, ExecutionError, MessageRole, PathBuf, SessionStore,
    StoredMessage, SystemTime, TcpStream, V1ErrorResponse, V1RunRequest, V1SessionCreateRequest,
    V1SessionCreateResponse, V1SessionShowResponse, V1SessionsListResponse, Value,
    SERVER_API_SCHEMA_VERSION, UNIX_EPOCH,
};

impl Engine {
    pub(crate) async fn handle_serve_run_request(
        &self,
        mut stream: TcpStream,
        body: Vec<u8>,
        serve_context: &CommandContext,
    ) -> Result<u16, ExecutionError> {
        let request: V1RunRequest = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "bad request".to_string(),
                    message: format!("invalid json body: {err}"),
                };
                serve_http::write_http_json(
                    &mut stream,
                    400,
                    "Bad Request",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(400);
            }
        };
        if request.prompt.trim().is_empty() {
            let payload = V1ErrorResponse {
                error: "bad request".to_string(),
                message: "prompt must not be empty".to_string(),
            };
            serve_http::write_http_json(
                &mut stream,
                400,
                "Bad Request",
                &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
            )
            .await?;
            return Ok(400);
        }
        if request.schema_version != SERVER_API_SCHEMA_VERSION {
            let payload = V1ErrorResponse {
                error: "bad request".to_string(),
                message: format!("unsupported schema_version={}", request.schema_version),
            };
            serve_http::write_http_json(
                &mut stream,
                400,
                "Bad Request",
                &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
            )
            .await?;
            return Ok(400);
        }

        let cwd = std::env::current_dir()
            .map_err(|err| ExecutionError::Executor(format!("failed to resolve cwd: {err}")))?;
        let workspace_root = serve_context.config.workspace_root.clone();
        let model = serve_context.config.model.clone();
        let requested_session_id = request.session_id.clone();

        let store = SessionStore::open_default();
        let create_new = requested_session_id.is_none();
        let session_result = tokio::task::spawn_blocking(move || {
            if let Some(session_id) = requested_session_id {
                let info = store.get_session(&session_id)?;
                Ok::<_, rustcode_state::StateError>(info)
            } else {
                store.create_session(None, None, &cwd, &workspace_root, &model)
            }
        })
        .await
        .map_err(|err| ExecutionError::Executor(format!("session init join error: {err}")))?;

        let mut persist = true;
        let session_id = match session_result {
            Ok(info) => info.id,
            Err(rustcode_state::StateError::NotFound(_)) => {
                let payload = V1ErrorResponse {
                    error: "not found".to_string(),
                    message: "session not found".to_string(),
                };
                serve_http::write_http_json(
                    &mut stream,
                    404,
                    "Not Found",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(404);
            }
            Err(rustcode_state::StateError::Validation(message)) => {
                let payload = V1ErrorResponse {
                    error: "bad request".to_string(),
                    message,
                };
                serve_http::write_http_json(
                    &mut stream,
                    400,
                    "Bad Request",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(400);
            }
            Err(err) if create_new => {
                persist = false;
                debug!("failed to create session for /v1/run: {err}");
                format!("session-{}", unix_ms())
            }
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "internal".to_string(),
                    message: err.to_string(),
                };
                serve_http::write_http_json(
                    &mut stream,
                    500,
                    "Internal Server Error",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(500);
            }
        };

        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nrustcode-session-id: {session_id}\r\nconnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await.map_err(|err| {
            ExecutionError::Executor(format!("failed to write sse headers: {err}"))
        })?;

        let sse_publisher = Arc::new(serve_http::SsePublisher {
            stream: tokio::sync::Mutex::new(stream),
        });

        let capture = Arc::new(tokio::sync::Mutex::new(String::new()));
        let publisher: Arc<dyn EventPublisher> = Arc::new(serve_http::CapturingPublisher {
            inner: sse_publisher,
            captured_output: capture.clone(),
        });

        let now_ms = unix_ms();
        let request_id = format!("request-{now_ms}");

        let cancellation = CancellationToken::new();
        let serve_cancel = serve_context.cancellation.clone();
        let cancellation_clone = cancellation.clone();
        tokio::spawn(async move {
            serve_cancel.cancelled().await;
            cancellation_clone.cancel();
        });

        let context = CommandContext::with_cancellation(
            serve_context.config.clone(),
            rustcode_core::context::SessionMeta {
                session_id: session_id.clone(),
                request_id,
                started_at: SystemTime::now(),
            },
            cancellation,
        );

        let prompt_for_store = request.prompt.clone();
        let command = Command::Run {
            prompt: request.prompt,
        };
        let result = self.execute(command, context, publisher).await;

        if persist && matches!(result, Ok(())) {
            let assistant = capture.lock().await.trim_end().to_string();
            let prompt = prompt_for_store;
            let model = serve_context.config.model.clone();
            let workspace_root = serve_context.config.workspace_root.clone();
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let store = SessionStore::open_default();
            let session_id = session_id.clone();
            tokio::task::spawn_blocking(move || {
                if matches!(
                    store.get_session(&session_id),
                    Err(rustcode_state::StateError::NotFound(_))
                ) {
                    let _ = store.create_session(None, None, &cwd, &workspace_root, &model);
                }
                let now = unix_ms_i64();
                let user = StoredMessage {
                    id: store.new_message_id(),
                    role: MessageRole::User,
                    created_at_unix_ms: now,
                    content: Value::String(prompt),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                };
                let assistant_msg = StoredMessage {
                    id: store.new_message_id(),
                    role: MessageRole::Assistant,
                    created_at_unix_ms: now,
                    content: Value::String(assistant),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                };
                let _ = store.append_message(&session_id, &user);
                let _ = store.append_message(&session_id, &assistant_msg);
            })
            .await
            .ok();
        }

        Ok(200)
    }

    pub(crate) async fn handle_serve_list_sessions(
        &self,
        stream: &mut TcpStream,
    ) -> Result<u16, ExecutionError> {
        let store = SessionStore::open_default();
        let root = store.root().display().to_string();
        let sessions_result = tokio::task::spawn_blocking(move || store.list_sessions())
            .await
            .map_err(|err| ExecutionError::Executor(format!("list sessions join error: {err}")))?;

        let sessions = match sessions_result {
            Ok(sessions) => sessions,
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "internal".to_string(),
                    message: err.to_string(),
                };
                serve_http::write_http_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(500);
            }
        };

        let payload = V1SessionsListResponse {
            schema_version: SERVER_API_SCHEMA_VERSION,
            sessions_root: root,
            sessions,
        };
        serve_http::write_http_json(
            stream,
            200,
            "OK",
            &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
        )
        .await?;
        Ok(200)
    }

    pub(crate) async fn handle_serve_create_session(
        &self,
        stream: &mut TcpStream,
        body: Vec<u8>,
        context: &CommandContext,
    ) -> Result<u16, ExecutionError> {
        let request: V1SessionCreateRequest = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "bad request".to_string(),
                    message: format!("invalid json body: {err}"),
                };
                serve_http::write_http_json(
                    stream,
                    400,
                    "Bad Request",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(400);
            }
        };
        if request.schema_version != SERVER_API_SCHEMA_VERSION {
            let payload = V1ErrorResponse {
                error: "bad request".to_string(),
                message: format!("unsupported schema_version={}", request.schema_version),
            };
            serve_http::write_http_json(
                stream,
                400,
                "Bad Request",
                &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
            )
            .await?;
            return Ok(400);
        }

        let title = request.title;
        let workspace_root = context.config.workspace_root.clone();
        let model = context.config.model.clone();
        let cwd = std::env::current_dir()
            .map_err(|err| ExecutionError::Executor(format!("failed to resolve cwd: {err}")))?;

        let store = SessionStore::open_default();
        let session_result = tokio::task::spawn_blocking(move || {
            store.create_session(title, None, &cwd, &workspace_root, &model)
        })
        .await
        .map_err(|err| ExecutionError::Executor(format!("create session join error: {err}")))?;

        let session = match session_result {
            Ok(session) => session,
            Err(rustcode_state::StateError::Validation(message)) => {
                let payload = V1ErrorResponse {
                    error: "bad request".to_string(),
                    message,
                };
                serve_http::write_http_json(
                    stream,
                    400,
                    "Bad Request",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(400);
            }
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "internal".to_string(),
                    message: err.to_string(),
                };
                serve_http::write_http_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                return Ok(500);
            }
        };

        let payload = V1SessionCreateResponse {
            schema_version: SERVER_API_SCHEMA_VERSION,
            session,
        };
        serve_http::write_http_json(
            stream,
            201,
            "Created",
            &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
        )
        .await?;
        Ok(201)
    }

    pub(crate) async fn handle_serve_show_session(
        &self,
        stream: &mut TcpStream,
        session_id: &str,
    ) -> Result<u16, ExecutionError> {
        let store = SessionStore::open_default();
        let session_id = session_id.to_string();
        let result: Result<_, rustcode_state::StateError> =
            tokio::task::spawn_blocking(move || {
                let session = store.get_session(&session_id)?;
                let messages = store.load_messages(&session_id)?;
                Ok::<_, rustcode_state::StateError>((session, messages))
            })
            .await
            .map_err(|err| ExecutionError::Executor(format!("show session join error: {err}")))?;

        match result {
            Ok((session, messages)) => {
                let payload = V1SessionShowResponse {
                    schema_version: SERVER_API_SCHEMA_VERSION,
                    session,
                    messages,
                };
                serve_http::write_http_json(
                    stream,
                    200,
                    "OK",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                Ok(200)
            }
            Err(rustcode_state::StateError::NotFound(_)) => {
                let payload = V1ErrorResponse {
                    error: "not found".to_string(),
                    message: "session not found".to_string(),
                };
                serve_http::write_http_json(
                    stream,
                    404,
                    "Not Found",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                Ok(404)
            }
            Err(rustcode_state::StateError::Validation(message)) => {
                let payload = V1ErrorResponse {
                    error: "bad request".to_string(),
                    message,
                };
                serve_http::write_http_json(
                    stream,
                    400,
                    "Bad Request",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                Ok(400)
            }
            Err(err) => {
                let payload = V1ErrorResponse {
                    error: "internal".to_string(),
                    message: err.to_string(),
                };
                serve_http::write_http_json(
                    stream,
                    500,
                    "Internal Server Error",
                    &(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()) + "\n"),
                )
                .await?;
                Ok(500)
            }
        }
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn unix_ms_i64() -> i64 {
    let ms = unix_ms();
    i64::try_from(ms).unwrap_or(i64::MAX)
}
