use super::{Engine, CommandContext, Arc, EventPublisher, ExecutionError, TcpListener, EventScope, EventPayload, TcpStream, serve_http};

impl Engine {
    pub(crate) async fn run_serve(
        &self,
        listen: String,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let listener = TcpListener::bind(&listen)
            .await
            .map_err(|err| ExecutionError::Executor(format!("failed to bind {listen}: {err}")))?;
        let bound_addr = listener
            .local_addr()
            .map_err(|err| ExecutionError::Executor(format!("failed to inspect bind addr: {err}")))?;

        self.emit(
            publisher.clone(),
            EventScope::System,
            EventPayload::Warning {
                message: format!("serve endpoint configured: {bound_addr}"),
            },
            context,
        )
        .await?;

        loop {
            tokio::select! {
                () = context.cancellation.cancelled() => {
                    return Err(ExecutionError::Cancelled);
                }
                incoming = listener.accept() => {
                    match incoming {
                        Ok((stream, _addr)) => {
                            if let Err(err) = self
                                .handle_serve_connection(stream, context, publisher.clone())
                                .await
                            {
                                self.emit(
                                    publisher.clone(),
                                    EventScope::System,
                                    EventPayload::Warning {
                                        message: format!("serve connection error: {err}"),
                                    },
                                    context,
                                )
                                .await?;
                            }
                        }
                        Err(err) => {
                            return Err(ExecutionError::Executor(format!("accept failed: {err}")));
                        }
                    }
                }
            }
        }
    }

    async fn handle_serve_connection(
        &self,
        mut stream: TcpStream,
        context: &CommandContext,
        publisher: Arc<dyn EventPublisher>,
    ) -> Result<(), ExecutionError> {
        let request = match serve_http::read_http_request(&mut stream).await {
            Ok(request) => request,
            Err(serve_http::ReadHttpRequestError::Timeout) => {
                serve_http::write_http_json(
                    &mut stream,
                    408,
                    "Request Timeout",
                    "{\"error\":\"request timeout\"}\n",
                )
                .await?;
                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::ServeRequest {
                        method: String::new(),
                        path: String::new(),
                        status: 408,
                    },
                    context,
                )
                .await?;
                return Ok(());
            }
            Err(serve_http::ReadHttpRequestError::BadRequest(message)) => {
                let payload = serde_json::json!({
                    "error": "bad request",
                    "message": message,
                })
                .to_string();
                serve_http::write_http_json(&mut stream, 400, "Bad Request", &(payload + "\n")).await?;
                self.emit(
                    publisher,
                    EventScope::System,
                    EventPayload::ServeRequest {
                        method: String::new(),
                        path: String::new(),
                        status: 400,
                    },
                    context,
                )
                .await?;
                return Ok(());
            }
            Err(serve_http::ReadHttpRequestError::Io(err)) => {
                return Err(ExecutionError::Executor(format!(
                    "failed to read request: {err}"
                )));
            }
        };

        let method = request.method.clone();
        let path = request.path.clone();

        if method == "GET" && path == "/v1/sessions" {
            let status = self.handle_serve_list_sessions(&mut stream).await?;
            self.emit(
                publisher,
                EventScope::System,
                EventPayload::ServeRequest {
                    method,
                    path,
                    status,
                },
                context,
            )
            .await?;
            return Ok(());
        }

        if method == "POST" && path == "/v1/sessions" {
            let status = self
                .handle_serve_create_session(&mut stream, request.body, context)
                .await?;
            self.emit(
                publisher,
                EventScope::System,
                EventPayload::ServeRequest {
                    method,
                    path,
                    status,
                },
                context,
            )
            .await?;
            return Ok(());
        }

        if method == "GET" {
            if let Some(session_id) = path.strip_prefix("/v1/sessions/") {
                if !session_id.is_empty() && !session_id.contains('/') {
                    let status = self.handle_serve_show_session(&mut stream, session_id).await?;
                    self.emit(
                        publisher,
                        EventScope::System,
                        EventPayload::ServeRequest {
                            method,
                            path,
                            status,
                        },
                        context,
                    )
                    .await?;
                    return Ok(());
                }
            }
        }

        if method == "POST" && path == "/v1/run" {
            let status = self.handle_serve_run_request(stream, request.body, context).await?;
            self.emit(
                publisher,
                EventScope::System,
                EventPayload::ServeRequest {
                    method,
                    path,
                    status,
                },
                context,
            )
            .await?;
            return Ok(());
        }

        let (status_code, status_text, body) = match (method.as_str(), path.as_str()) {
            ("GET", "/health") => (200u16, "OK", "{\"ok\":true}\n"),
            _ => (404u16, "Not Found", "{\"error\":\"not found\"}\n"),
        };
        serve_http::write_http_json(&mut stream, status_code, status_text, body).await?;

        self.emit(
            publisher,
            EventScope::System,
            EventPayload::ServeRequest {
                method,
                path,
                status: status_code,
            },
            context,
        )
        .await
    }
}
