use super::*;

#[test]
fn version_does_not_require_trusted_project_config_or_llm_init() {
    let seed = make_temp_file_path("version-untrusted");
    let stem = seed
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("rustcode-version-untrusted");
    let project_root = std::env::temp_dir().join(stem);
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).expect("create project dir");

    // Create an untrusted project config that would normally trip the trust gate.
    std::fs::write(project_root.join("rustcode.toml"), "allow_network = true\n")
        .expect("write project config");

    let output = Command::new(rustcode_bin())
        .current_dir(&project_root)
        .arg("version")
        .output()
        .expect("run rustcode version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().starts_with("0."), "stdout={stdout}");

    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn version_flag_does_not_require_trusted_project_config_or_llm_init() {
    let seed = make_temp_file_path("version-flag-untrusted");
    let stem = seed
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("rustcode-version-flag-untrusted");
    let project_root = std::env::temp_dir().join(stem);
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).expect("create project dir");

    // Create an untrusted project config that would normally trip the trust gate.
    std::fs::write(project_root.join("rustcode.toml"), "allow_network = true\n")
        .expect("write project config");

    let output = Command::new(rustcode_bin())
        .current_dir(&project_root)
        .arg("--version")
        .output()
        .expect("run rustcode --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0."), "stdout={stdout}");

    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn list_does_not_require_llm_provider_even_when_allow_network_true() {
    let seed = make_temp_file_path("list-no-llm-init");
    let stem = seed
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("rustcode-list-no-llm-init");
    let project_root = std::env::temp_dir().join(stem);
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).expect("create project dir");

    // allow_network=true would previously cause provider policy selection and a missing-api-key error
    // during CLI startup, even for non-LLM commands like `list`.
    std::fs::write(project_root.join("rustcode.toml"), "allow_network = true\n")
        .expect("write project config");

    let output = Command::new(rustcode_bin())
        .current_dir(&project_root)
        .arg("--trust-project-config")
        .arg("list")
        .arg(".")
        .output()
        .expect("run rustcode list");
    assert!(output.status.success());

    let _ = std::fs::remove_dir_all(&project_root);
}

#[test]
fn json_stream_includes_schema_version_and_completion_event() {
    let sessions_dir = make_temp_dir_path("sessions-json-stream");
    let output = Command::new(rustcode_bin())
        .args(["--deny-network", "--json", "run", "integration-stream"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
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
fn human_run_output_is_plain_text_by_default() {
    let sessions_dir = make_temp_dir_path("sessions-human-run");
    let output = Command::new(rustcode_bin())
        .args(["--deny-network", "run", "integration-human"])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("null-llm response"));
    assert!(
        !stdout.contains("OutputChunk"),
        "stdout should be coalesced"
    );
}

#[test]
fn run_attach_streams_output_chunks_from_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut buf = [0_u8; 8192];
        let n = socket.read(&mut buf).expect("read");
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(request.contains("POST /v1/run"), "request={request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("accept: text/event-stream"),
            "request={request}"
        );
        assert!(
            request.contains("\"prompt\":\"hello\""),
            "request={request}"
        );

        let chunk_event = Event::new(
            1,
            EventScope::Command,
            EventPayload::OutputChunk {
                text: "hello-from-attach".to_string(),
            },
        );
        let completed_event = Event::new(2, EventScope::Command, EventPayload::Completed);
        let chunk_json = serde_json::to_string(&chunk_event).expect("chunk json");
        let completed_json = serde_json::to_string(&completed_event).expect("completed json");

        let body = format!("data: {chunk_json}\n\ndata: {completed_json}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).expect("write");
        socket.flush().expect("flush");
    });

    let output = Command::new(rustcode_bin())
        .args([
            "--allow-network",
            "run",
            "--attach",
            &format!("http://127.0.0.1:{port}"),
            "hello",
        ])
        .output()
        .expect("must run rustcode binary");

    handle.join().expect("server should join");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert_eq!(stdout.trim_end(), "hello-from-attach");
}

#[test]
fn human_run_output_uses_event_envelope_with_event_debug() {
    let sessions_dir = make_temp_dir_path("sessions-human-run-debug");
    let output = Command::new(rustcode_bin())
        .args([
            "--deny-network",
            "--event-debug",
            "run",
            "integration-human-debug",
        ])
        .env("RUSTCODE_SESSIONS_DIR", &sessions_dir)
        .output()
        .expect("must run rustcode binary");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");
    assert!(stdout.contains("OutputChunk"));
    assert!(stdout.contains("Completed"));
}
