use super::*;

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

#[cfg(unix)]
#[test]
fn sigint_cancels_hanging_llm_request_gracefully() {
    let Some((port, handle)) = spawn_hanging_http_server() else {
        return;
    };

    let sessions_dir = make_temp_dir_path("sessions-sigint-hanging-llm");

    let config_path = make_temp_file_path("sigint-hanging-llm");
    std::fs::write(
        &config_path,
        format!(
            r#"
allow_network = true
model = "openai/gpt-5"

[llm]
provider = "openai"
base_url = "http://127.0.0.1:{port}"
api_key_env = "RUSTCODE_TEST_KEY"
"#
        ),
    )
    .expect("must write config fixture");

    let mut child = Command::new(rustcode_bin())
        .args(["run", "hang"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .env("RUSTCODE_USER_CONFIG", &config_path)
        .env("RUSTCODE_TRUST_PROJECT", "0")
        .env("RUSTCODE_TEST_KEY", "integration-secret")
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
    thread::sleep(Duration::from_millis(500));

    let pid = child.id().to_string();
    let kill_status = Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("must invoke kill");
    assert!(kill_status.success(), "failed to send SIGINT");

    let output = child
        .wait_with_output()
        .expect("must collect rustcode output");
    handle.join().expect("server should join");

    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("execution cancelled"), "stdout: {stdout}");
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
