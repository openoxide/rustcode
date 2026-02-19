// ── PTY process execution ──────────────────────────────────────────────
//
// Spawns a subprocess under a real pseudo-terminal (PTY) so commands that
// inspect `isatty()` behave as they would in a normal terminal session.
//
// Design: one-shot execution (no persistent session), agent-focused.
// Reference: codex `utils/pty/pty.rs`.

use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::IoError;

/// Default PTY column count (wide enough for most output).
const DEFAULT_COLS: u16 = 220;
/// Default PTY row count.
const DEFAULT_ROWS: u16 = 50;
/// Default execution timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// Maximum output bytes before truncation (512 KB).
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
/// Read buffer size for PTY reader loop.
const READ_BUF_SIZE: usize = 8 * 1024;

/// Output from a PTY-spawned process.
#[derive(Debug, Clone)]
pub struct PtyOutput {
    /// Merged stdout+stderr as the terminal would display them.
    /// ANSI escape sequences are stripped before returning.
    pub output: String,
    /// Process exit code.
    pub exit_code: i32,
}

/// Spawn `program` with `args` in a PTY and collect all output.
///
/// Uses default PTY size (220×50) and a 5-minute timeout.
/// Output is capped at 512 KB; ANSI escape sequences are stripped.
///
/// # Errors
/// Returns `IoError::Cancelled` if the cancellation token fires,
/// `IoError::Io` for spawn or read failures.
pub async fn run_pty(
    program: &str,
    args: &[String],
    cwd: &Path,
    cancellation: CancellationToken,
) -> Result<PtyOutput, IoError> {
    run_pty_with_opts(
        program,
        args,
        cwd,
        DEFAULT_COLS,
        DEFAULT_ROWS,
        cancellation,
        DEFAULT_TIMEOUT,
    )
    .await
}

/// Spawn `program` in a PTY with explicit size and timeout.
pub async fn run_pty_with_opts(
    program: &str,
    args: &[String],
    cwd: &Path,
    cols: u16,
    rows: u16,
    cancellation: CancellationToken,
    timeout: Duration,
) -> Result<PtyOutput, IoError> {
    let program = program.to_string();
    let args = args.to_vec();
    let cwd = cwd.to_path_buf();

    // Spawn the PTY in a blocking task (portable_pty uses blocking I/O)
    let (output_tx, mut output_rx) = mpsc::channel::<Result<Vec<u8>, IoError>>(256);

    let spawn_handle = tokio::task::spawn_blocking(move || {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| IoError::Io(format!("failed to open PTY: {e}")))?;

        let mut cmd = CommandBuilder::new(&program);
        for arg in &args {
            cmd.arg(arg);
        }
        cmd.cwd(&cwd);

        // Spawn the child process inside the PTY slave
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| IoError::Io(format!("failed to spawn PTY command: {e}")))?;

        // Drop the slave side now that the child owns it
        drop(pair.slave);

        // Read all output from the master side
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| IoError::Io(format!("failed to clone PTY reader: {e}")))?;

        let mut buf = [0u8; READ_BUF_SIZE];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break, // EOF — child has exited and master is closed
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    // If receiver dropped (cancelled), stop reading
                    if output_tx.blocking_send(Ok(chunk)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // EIO is normal on Linux when the slave is closed
                    let raw = e.raw_os_error();
                    if raw == Some(5) || raw == Some(6) {
                        break; // EIO / ENXIO — child closed the PTY
                    }
                    let _ = output_tx.blocking_send(Err(IoError::Io(e.to_string())));
                    break;
                }
            }
        }

        // portable_pty::ExitStatus has exit_code() -> u32, not .code()/.signal()
        let exit_code = child.wait().map_or(-1, |status| status.exit_code() as i32);

        Ok::<i32, IoError>(exit_code)
    });

    // Collect output with timeout + cancellation
    let mut raw_bytes: Vec<u8> = Vec::new();
    let mut read_error: Option<IoError> = None;

    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            () = cancellation.cancelled() => {
                return Err(IoError::Cancelled);
            }
            () = &mut deadline => {
                // Timeout — stop collecting, use what we have
                break;
            }
            msg = output_rx.recv() => {
                match msg {
                    None => break, // channel closed, spawn_blocking finished
                    Some(Ok(chunk)) => {
                        raw_bytes.extend_from_slice(&chunk);
                        if raw_bytes.len() >= MAX_OUTPUT_BYTES {
                            // Cap reached — drain remaining without storing
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        read_error = Some(e);
                        break;
                    }
                }
            }
        }
    }

    if let Some(e) = read_error {
        return Err(e);
    }

    let exit_code = spawn_handle
        .await
        .map_err(|e| IoError::Io(format!("PTY task panicked: {e}")))?
        .unwrap_or(-1);

    // Decode bytes, strip ANSI, truncate
    let raw_str = String::from_utf8_lossy(&raw_bytes).into_owned();
    let stripped = strip_ansi(&raw_str);
    let output = if stripped.len() > MAX_OUTPUT_BYTES {
        truncate_pty_output(&stripped, MAX_OUTPUT_BYTES)
    } else {
        stripped
    };

    Ok(PtyOutput { output, exit_code })
}

/// Strip ANSI escape sequences from a string.
///
/// Removes CSI sequences (`ESC[...m`) and OSC sequences (`ESC]...ST`).
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next(); // consume '['
                                  // Skip until a final byte (0x40–0x7E)
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next(); // consume ']'
                                  // Skip until ST (ESC \) or BEL
                    loop {
                        match chars.next() {
                            Some('\x07') => break,
                            Some('\x1b') => {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
                _ => {} // Unknown escape — skip the ESC, keep the next char
            }
        } else {
            out.push(ch);
        }
    }

    out
}

/// Truncate PTY output preserving head and tail with a marker in the middle.
fn truncate_pty_output(output: &str, max_bytes: usize) -> String {
    let keep = max_bytes / 2;
    let head = &output[..keep.min(output.len())];
    let tail_start = output.len().saturating_sub(keep);
    let tail = &output[tail_start..];
    let omitted = output.len().saturating_sub(max_bytes);
    format!("{head}\n\n[rustcode: output truncated — {omitted} bytes omitted]\n\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_csi() {
        let input = "\x1b[32mhello\x1b[0m world";
        assert_eq!(strip_ansi(input), "hello world");
    }

    #[test]
    fn strip_ansi_passthrough_plain() {
        let input = "plain text\nno escapes";
        assert_eq!(strip_ansi(input), input);
    }

    #[test]
    fn strip_ansi_removes_osc() {
        // OSC sequence ending with BEL
        let input = "\x1b]0;window title\x07text";
        assert_eq!(strip_ansi(input), "text");
    }

    #[test]
    fn truncate_pty_output_inserts_marker() {
        let data = "A".repeat(200);
        let result = truncate_pty_output(&data, 100);
        assert!(result.contains("[rustcode: output truncated"));
        assert!(result.starts_with("AAAA"));
    }

    #[tokio::test]
    async fn run_pty_executes_echo() {
        let cwd = std::env::temp_dir();
        let cancel = CancellationToken::new();
        let result = run_pty("echo", &["hello from pty".to_string()], &cwd, cancel).await;
        match result {
            Ok(out) => {
                assert!(
                    out.output.contains("hello from pty"),
                    "unexpected output: {:?}",
                    out.output
                );
                assert_eq!(out.exit_code, 0);
            }
            Err(e) => panic!("pty exec failed: {e}"),
        }
    }

    #[tokio::test]
    async fn run_pty_nonzero_exit_preserved() {
        let cwd = std::env::temp_dir();
        let cancel = CancellationToken::new();
        let result = run_pty(
            "/bin/sh",
            &["-c".to_string(), "exit 42".to_string()],
            &cwd,
            cancel,
        )
        .await;
        match result {
            Ok(out) => assert_eq!(out.exit_code, 42),
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn run_pty_cancellation() {
        let cwd = std::env::temp_dir();
        let cancel = CancellationToken::new();
        // Cancel immediately before the call resolves
        let cancel_clone = cancel.clone();
        cancel_clone.cancel();
        let result = run_pty("sleep", &["10".to_string()], &cwd, cancel).await;
        assert!(matches!(result, Err(IoError::Cancelled)));
    }
}
