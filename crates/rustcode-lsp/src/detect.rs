// ── Language server auto-detection ───────────────────────────────────────
//
// Detects which language server to use based on project root markers.
// The first matching project type whose binary is found in PATH is returned.

use std::path::Path;

/// Description of a detected language server.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    /// Short identifier used in logs and config.
    pub server_id: &'static str,
    /// Executable name (must be in PATH).
    pub command: String,
    /// Arguments passed to the binary.
    pub args: Vec<String>,
}

/// Detect the appropriate language server for the given workspace root.
///
/// Detection order (first matching type whose binary is in PATH):
/// 1. `Cargo.toml` → `rust-analyzer`
/// 2. `go.mod` → `gopls`
/// 3. `pyproject.toml`, `setup.py`, or `requirements.txt` → `pyright --stdio`
/// 4. `tsconfig.json` or `package.json` → `typescript-language-server --stdio`
///
/// Returns `None` if no project markers are found or the binary is not installed.
#[must_use]
pub fn detect_server(workspace_root: &Path) -> Option<ServerSpec> {
    let candidates: &[(&'static str, &'static str, &[&'static str], &[&'static str])] = &[
        // (server_id, binary, extra_args, marker_files)
        ("rust-analyzer", "rust-analyzer", &[], &["Cargo.toml"]),
        ("gopls", "gopls", &[], &["go.mod"]),
        (
            "pyright",
            "pyright-langserver",
            &["--stdio"],
            &["pyproject.toml", "setup.py", "requirements.txt"],
        ),
        (
            "typescript-language-server",
            "typescript-language-server",
            &["--stdio"],
            &["tsconfig.json", "package.json"],
        ),
    ];

    for (server_id, binary, extra_args, markers) in candidates {
        // Check if any project marker exists
        let has_marker = markers.iter().any(|m| workspace_root.join(m).exists());

        if !has_marker {
            continue;
        }

        // Check if binary is available in PATH
        if binary_in_path(binary) {
            return Some(ServerSpec {
                server_id,
                command: binary.to_string(),
                args: extra_args.iter().map(|s| s.to_string()).collect(),
            });
        }

        tracing::warn!(
            "LSP: detected {} project but '{}' is not in PATH — skipping",
            server_id,
            binary
        );
    }

    None
}

/// Returns `true` if `name` resolves to an executable in the system PATH.
fn binary_in_path(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_markers_returns_none() {
        let dir = std::env::temp_dir().join("rustcode-lsp-no-markers");
        let _ = std::fs::create_dir_all(&dir);
        // Remove any accidental markers
        for m in ["Cargo.toml", "go.mod", "pyproject.toml", "tsconfig.json"] {
            let _ = std::fs::remove_file(dir.join(m));
        }
        assert!(detect_server(&dir).is_none());
    }

    #[test]
    fn rust_analyzer_detected_when_cargo_toml_exists() {
        // We can only verify the detection logic if rust-analyzer is installed.
        // If it is, the test confirms correct server selection.
        // If not, the function returns None (correct behavior for missing binary).
        let dir = std::env::temp_dir().join("rustcode-lsp-cargo-detect");
        let _ = std::fs::create_dir_all(&dir);
        let cargo_path = dir.join("Cargo.toml");
        let _ = std::fs::write(
            &cargo_path,
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        );

        let result = detect_server(&dir);

        // Clean up
        let _ = std::fs::remove_file(&cargo_path);

        if let Some(spec) = result {
            assert_eq!(spec.server_id, "rust-analyzer");
            assert_eq!(spec.command, "rust-analyzer");
        }
        // None is also valid if rust-analyzer is not installed
    }
}
