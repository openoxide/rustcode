use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::AuthError;

#[derive(Debug, Clone)]
pub(super) struct CallbackRoute {
    host: String,
    port: u16,
    path: String,
}

pub(super) fn callback_route_from_redirect_uri(
    redirect_uri: &str,
) -> Result<CallbackRoute, AuthError> {
    let parsed = reqwest::Url::parse(redirect_uri)
        .map_err(|err| AuthError::Validation(format!("invalid redirect uri: {err}")))?;
    let host = parsed.host_str().ok_or_else(|| {
        AuthError::Validation("redirect uri does not contain a callback host".to_string())
    })?;
    let port = parsed.port_or_known_default().ok_or_else(|| {
        AuthError::Validation("redirect uri does not contain a callback port".to_string())
    })?;
    let path = if parsed.path().is_empty() {
        "/".to_string()
    } else {
        parsed.path().to_string()
    };
    Ok(CallbackRoute {
        host: host.to_string(),
        port,
        path,
    })
}

pub(super) async fn wait_for_oauth_callback(
    callback: &CallbackRoute,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, AuthError> {
    let listener = TcpListener::bind((callback.host.as_str(), callback.port))
        .await
        .map_err(|err| {
            AuthError::Network(format!(
                "failed to bind oauth callback {}:{}: {err}",
                callback.host, callback.port
            ))
        })?;
    let deadline = tokio::time::Instant::now() + timeout.max(Duration::from_secs(1));

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(AuthError::OAuthTimeout);
        }
        let remaining = deadline - now;
        let accepted = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| AuthError::OAuthTimeout)?
            .map_err(|err| AuthError::Network(format!("oauth callback accept failed: {err}")))?;
        let (mut socket, _) = accepted;
        if let Some(code) = Box::pin(handle_oauth_callback_connection(
            &mut socket,
            expected_state,
            &callback.path,
        ))
        .await?
        {
            return Ok(code);
        }
    }
}

async fn handle_oauth_callback_connection(
    socket: &mut tokio::net::TcpStream,
    expected_state: &str,
    expected_path: &str,
) -> Result<Option<String>, AuthError> {
    let mut buffer = [0_u8; 16 * 1024];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
        .await
        .map_err(|_| AuthError::Network("oauth callback read timed out".to_string()))?
        .map_err(|err| AuthError::Network(format!("oauth callback read failed: {err}")))?;
    if read == 0 {
        return Ok(None);
    }

    let request = String::from_utf8_lossy(&buffer[..read]);
    let first_line = request.lines().next().ok_or_else(|| {
        AuthError::Parse("oauth callback request was missing request line".to_string())
    })?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");

    if method != "GET" {
        write_callback_response(socket, "405 Method Not Allowed", "method not allowed").await?;
        return Ok(None);
    }

    let parsed_url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|err| AuthError::Parse(format!("oauth callback url parse failed: {err}")))?;
    if parsed_url.path() != expected_path {
        write_callback_response(socket, "404 Not Found", "not found").await?;
        return Ok(None);
    }

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;
    let mut error_description: Option<String> = None;
    for (key, value) in parsed_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(reason) = error {
        let detail = error_description.unwrap_or_else(|| "oauth authorization failed".to_string());
        write_callback_response(socket, "400 Bad Request", "authorization failed").await?;
        return Err(AuthError::OAuthFailed(format!("{reason}: {detail}")));
    }

    if state.as_deref() != Some(expected_state) {
        write_callback_response(socket, "400 Bad Request", "invalid oauth state").await?;
        return Err(AuthError::OAuthFailed(
            "oauth callback contained invalid state".to_string(),
        ));
    }

    let code = code.ok_or_else(|| {
        AuthError::Parse("oauth callback did not include authorization code".to_string())
    })?;
    write_callback_response(
        socket,
        "200 OK",
        "authorization complete; return to rustcode terminal",
    )
    .await?;
    Ok(Some(code))
}

async fn write_callback_response(
    socket: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> Result<(), AuthError> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| AuthError::Network(format!("oauth callback write failed: {err}")))?;
    Ok(())
}
