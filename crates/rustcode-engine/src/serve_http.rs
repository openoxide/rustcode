use super::{async_trait, TcpStream, timeout, Duration, AsyncReadExt, StreamExt, ExecutionError, AsyncWriteExt, EventPublisher, Event, PublishError, Arc, EventPayload};

pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) enum ReadHttpRequestError {
    Timeout,
    BadRequest(String),
    Io(std::io::Error),
}

pub(crate) async fn read_http_request(
    stream: &mut TcpStream,
) -> Result<HttpRequest, ReadHttpRequestError> {
    const MAX_HEADER_BYTES: usize = 16 * 1024;
    const MAX_BODY_BYTES: usize = 1024 * 1024;

    let mut buffer = Vec::with_capacity(2048);
    let mut temp = [0u8; 2048];

    let header_end = loop {
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(ReadHttpRequestError::BadRequest(
                "request headers too large".to_string(),
            ));
        }
        match timeout(Duration::from_secs(2), stream.read(&mut temp)).await {
            Ok(Ok(0)) => {
                return Err(ReadHttpRequestError::BadRequest("empty request".to_string()));
            }
            Ok(Ok(n)) => {
                buffer.extend_from_slice(&temp[..n]);
                if let Some(pos) = find_header_end(&buffer) {
                    break pos;
                }
            }
            Ok(Err(err)) => return Err(ReadHttpRequestError::Io(err)),
            Err(_) => return Err(ReadHttpRequestError::Timeout),
        }
    };

    let (header_bytes, rest) = buffer.split_at(header_end);
    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|_| ReadHttpRequestError::BadRequest("headers must be utf-8".to_string()))?;
    let mut lines = header_str.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ReadHttpRequestError::BadRequest("missing request line".to_string()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| ReadHttpRequestError::BadRequest("missing method".to_string()))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| ReadHttpRequestError::BadRequest("missing path".to_string()))?
        .to_string();

    let mut headers = std::collections::BTreeMap::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(ReadHttpRequestError::BadRequest(
            "request body too large".to_string(),
        ));
    }

    let mut body = Vec::with_capacity(content_length);
    if content_length > 0 {
        let already = rest.len().min(content_length);
        body.extend_from_slice(&rest[..already]);
        while body.len() < content_length {
            let remaining = content_length - body.len();
            let chunk = remaining.min(temp.len());
            let n = stream
                .read(&mut temp[..chunk])
                .await
                .map_err(ReadHttpRequestError::Io)?;
            if n == 0 {
                return Err(ReadHttpRequestError::BadRequest(
                    "unexpected EOF reading body".to_string(),
                ));
            }
            body.extend_from_slice(&temp[..n]);
        }
    }

    Ok(HttpRequest { method, path, body })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

pub(crate) async fn write_http_json(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    body: &str,
) -> Result<(), ExecutionError> {
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|err| ExecutionError::Executor(format!("failed to write response: {err}")))?;
    stream
        .shutdown()
        .await
        .map_err(|err| ExecutionError::Executor(format!("failed to shutdown stream: {err}")))?;
    Ok(())
}

pub(crate) struct SsePublisher {
    pub(crate) stream: tokio::sync::Mutex<TcpStream>,
}

#[async_trait]
impl EventPublisher for SsePublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        let json = serde_json::to_string(&event).map_err(|_| PublishError::SinkClosed)?;
        let frame = format!("data: {json}\n\n");
        let mut stream = self.stream.lock().await;
        stream
            .write_all(frame.as_bytes())
            .await
            .map_err(|_| PublishError::SinkClosed)?;
        stream.flush().await.map_err(|_| PublishError::SinkClosed)?;
        Ok(())
    }
}

pub(crate) struct CapturingPublisher {
    pub(crate) inner: Arc<dyn EventPublisher>,
    pub(crate) captured_output: Arc<tokio::sync::Mutex<String>>,
}

#[async_trait]
impl EventPublisher for CapturingPublisher {
    async fn publish(&self, event: Event) -> Result<(), PublishError> {
        if let EventPayload::OutputChunk { text } = &event.payload {
            let mut out = self.captured_output.lock().await;
            out.push_str(text);
        }
        self.inner.publish(event).await
    }
}
