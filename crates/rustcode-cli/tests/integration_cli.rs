use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

fn rustcode_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rustcode-cli")
}

fn make_temp_file_path(name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    let pid = std::process::id();
    std::env::temp_dir().join(format!("rustcode-cli-{name}-{pid}-{now}.json"))
}

fn http_request(port: u16, path: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

#[test]
fn json_stream_includes_schema_version_and_completion_event() {
    let output = Command::new(rustcode_bin())
        .args(["--json", "run", "integration-stream"])
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.len() >= 2, "expected streamed event lines");

    let mut saw_completed = false;
    for line in lines {
        let value: Value = serde_json::from_str(line).expect("line must be valid json");
        assert_eq!(value["schema_version"].as_u64(), Some(1));

        if value["payload"]["type"].as_str() == Some("Completed") {
            saw_completed = true;
        }
    }

    assert!(saw_completed, "expected Completed event in stream");
}

#[test]
fn auth_set_key_and_status_round_trip() {
    let auth_path = make_temp_file_path("auth-store");

    let set_output = Command::new(rustcode_bin())
        .args([
            "auth",
            "set-key",
            "openrouter",
            "--from-env",
            "RUSTCODE_TEST_KEY",
        ])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .env("RUSTCODE_TEST_KEY", "integration-secret")
        .output()
        .expect("must run rustcode auth set-key");

    assert!(
        set_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&set_output.stdout),
        String::from_utf8_lossy(&set_output.stderr)
    );

    let status_output = Command::new(rustcode_bin())
        .args(["auth", "status", "openrouter"])
        .env("RUSTCODE_AUTH_FILE", &auth_path)
        .output()
        .expect("must run rustcode auth status");

    assert!(
        status_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status_output.stdout),
        String::from_utf8_lossy(&status_output.stderr)
    );
    let status_stdout = String::from_utf8(status_output.stdout).expect("stdout must be utf8");
    assert!(status_stdout.contains("provider=openrouter"));
    assert!(status_stdout.contains("credential=stored:api_key"));
}

#[test]
fn models_command_reads_custom_models_index() {
    let models_path = make_temp_file_path("models-index");
    std::fs::write(
        &models_path,
        r#"{
  "alpha": { "name": "Alpha Provider", "models": { "m1": {}, "m2": {} } },
  "beta": { "name": "Beta Provider", "models": { "x1": {} } }
}"#,
    )
    .expect("must write models fixture");

    let output = Command::new(rustcode_bin())
        .args(["models", "alpha"])
        .env("RUSTCODE_MODELS_PATH", &models_path)
        .output()
        .expect("must run rustcode models");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("alpha/m1"), "stdout: {stdout}");
    assert!(stdout.contains("alpha/m2"), "stdout: {stdout}");
}

#[cfg(unix)]
#[test]
fn sigint_cancels_long_running_command_gracefully() {
    let mut child = Command::new(rustcode_bin())
        .args(["exec", "sleep", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("must spawn rustcode process");

    // Avoid signal race by ensuring process remains alive briefly before SIGINT.
    let mut saw_running = false;
    for _ in 0..20 {
        match child.try_wait().expect("must query child state") {
            None => {
                saw_running = true;
                break;
            }
            Some(status) => {
                panic!("child exited before signal with status: {status}");
            }
        }
    }
    assert!(saw_running, "child never reached running state");
    thread::sleep(Duration::from_millis(1000));

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("must invoke kill");
    assert!(kill_status.success(), "failed to send SIGINT");

    let output = child
        .wait_with_output()
        .expect("must collect rustcode output");

    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("execution cancelled"));
}

#[test]
fn tui_command_routes_to_tui_consumer_loop() {
    let output = Command::new(rustcode_bin())
        .arg("tui")
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(
        stdout.trim().is_empty(),
        "tui mode should not print CLI renderer output"
    );
}

#[cfg(unix)]
#[test]
fn serve_exposes_health_response_and_cancels_cleanly() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            // Some sandboxed environments disallow local TCP binds.
            return;
        }
        Err(err) => panic!("must reserve port: {err}"),
    };
    let port = listener.local_addr().expect("must read local addr").port();
    drop(listener);

    let listen = format!("127.0.0.1:{port}");
    let child = Command::new(rustcode_bin())
        .args(["serve", "--listen", &listen])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("must spawn rustcode serve process");

    let mut health_response = String::new();
    let mut connected = false;
    for _ in 0..30 {
        match http_request(port, "/health") {
            Ok(response) => {
                health_response = response;
                connected = true;
                break;
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }

    assert!(connected, "failed to connect to serve endpoint");
    assert!(
        health_response.contains("200 OK"),
        "response: {health_response}"
    );
    assert!(
        health_response.contains("{\"ok\":true}"),
        "response: {health_response}"
    );

    let not_found_response = http_request(port, "/does-not-exist").expect("must read 404 response");
    assert!(
        not_found_response.contains("404 Not Found"),
        "response: {not_found_response}"
    );
    assert!(
        not_found_response.contains("{\"error\":\"not found\"}"),
        "response: {not_found_response}"
    );

    let mut timeout_stream =
        TcpStream::connect(("127.0.0.1", port)).expect("must open timeout probe connection");
    timeout_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("must set timeout");
    let mut timeout_response = String::new();
    timeout_stream
        .read_to_string(&mut timeout_response)
        .expect("must read timeout response");
    assert!(
        timeout_response.contains("408 Request Timeout"),
        "response: {timeout_response}"
    );
    assert!(
        timeout_response.contains("{\"error\":\"request timeout\"}"),
        "response: {timeout_response}"
    );

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("must invoke kill");
    assert!(kill_status.success(), "failed to send SIGINT");

    let output = child
        .wait_with_output()
        .expect("must collect rustcode output");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(
        stdout.contains("serve endpoint configured"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"GET\", path: \"/health\", status: 200 }"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"GET\", path: \"/does-not-exist\", status: 404 }"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("ServeRequest { method: \"\", path: \"\", status: 408 }"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("execution cancelled"), "stdout: {stdout}");
}
