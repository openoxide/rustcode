use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

fn rustcode_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rustcode-cli")
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

#[cfg(unix)]
#[test]
fn sigint_cancels_long_running_command_gracefully() {
    let child = Command::new(rustcode_bin())
        .args(["exec", "sleep", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("must spawn rustcode process");

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

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
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
