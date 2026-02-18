use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use rustcode_core::event::{Event, EventPayload, EventScope};
use serde_json::Value;

fn rustcode_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rustcode")
}

fn make_temp_file_path(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("rustcode-{name}-{pid}-{now}.json"))
}

fn make_temp_dir_path(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("rustcode-{name}-{pid}-{now}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn http_request(port: u16, path: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn spawn_mcp_discovery_server() -> Option<(u16, thread::JoinHandle<()>)> {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(_) => return None,
    };
    let port = listener.local_addr().ok()?.port();
    let handle = thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buf = [0_u8; 2048];
            let _ = socket.read(&mut buf);
            let body = r#"{"authorization_endpoint":"https://mcp.example.com/authorize","token_endpoint":"https://mcp.example.com/token"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        }
    });
    Some((port, handle))
}

fn spawn_hanging_http_server() -> Option<(u16, thread::JoinHandle<()>)> {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return None,
        Err(_) => return None,
    };
    let port = listener.local_addr().ok()?.port();
    let handle = thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            // Read the request so the client has completed the write, then hang until the socket
            // closes (SIGINT path should drop the request future and close the connection).
            let mut buf = [0_u8; 8192];
            let _ = socket.read(&mut buf);
            loop {
                match socket.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        }
    });
    Some((port, handle))
}

#[path = "integration_cli/auth_browser_login.rs"]
mod auth_browser_login;
#[path = "integration_cli/auth_login_methods.rs"]
mod auth_login_methods;
#[path = "integration_cli/auth_store_status.rs"]
mod auth_store_status;
#[path = "integration_cli/core_run.rs"]
mod core_run;
#[path = "integration_cli/mcp_config_status.rs"]
mod mcp_config_status;
#[path = "integration_cli/mcp_login.rs"]
mod mcp_login;
#[path = "integration_cli/models.rs"]
mod models;
#[path = "integration_cli/session_github.rs"]
mod session_github;
#[path = "integration_cli/signal_serve.rs"]
mod signal_serve;
