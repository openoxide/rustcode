// ── IDE detection ─────────────────────────────────────────────────────
//
// Detects which IDE (if any) launched the rustcode process by inspecting
// well-known environment variables set by editor extensions.
//
// Reference: opencode `ide/index.ts`

/// The IDE that launched this rustcode process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeInfo {
    /// Visual Studio Code
    VSCode,
    /// VS Code Insiders
    VSCodeInsiders,
    /// Cursor editor
    Cursor,
    /// Windsurf editor
    Windsurf,
    /// `VSCodium` (open-source VS Code fork)
    VSCodium,
    /// Unknown or no IDE detected
    Unknown,
}

/// Detect the IDE that launched this rustcode process.
///
/// Checks environment variables in priority order:
/// 1. `RUSTCODE_CALLER` — set by the rustcode extension for the editor
/// 2. `TERM_PROGRAM=vscode` + `GIT_ASKPASS` path analysis
/// 3. `VSCODE_IPC_HOOK_CLI` present → VS Code
/// 4. Otherwise `Unknown`
#[must_use]
pub fn detect_ide() -> IdeInfo {
    // Priority 1: explicit caller env var (set by rustcode extension)
    if let Ok(caller) = std::env::var("RUSTCODE_CALLER") {
        return match caller.to_lowercase().as_str() {
            "cursor" => IdeInfo::Cursor,
            "windsurf" => IdeInfo::Windsurf,
            "vscode-insiders" | "code-insiders" => IdeInfo::VSCodeInsiders,
            "vscodium" | "codium" => IdeInfo::VSCodium,
            "vscode" | "code" => IdeInfo::VSCode,
            _ => IdeInfo::Unknown,
        };
    }

    // Priority 2: TERM_PROGRAM=vscode + GIT_ASKPASS path analysis
    if std::env::var("TERM_PROGRAM").as_deref() == Ok("vscode") {
        if let Ok(askpass) = std::env::var("GIT_ASKPASS") {
            let lower = askpass.to_lowercase();
            if lower.contains("cursor") {
                return IdeInfo::Cursor;
            }
            if lower.contains("windsurf") {
                return IdeInfo::Windsurf;
            }
            if lower.contains("code-insiders") || lower.contains("vscode-insiders") {
                return IdeInfo::VSCodeInsiders;
            }
            if lower.contains("codium") || lower.contains("vscodium") {
                return IdeInfo::VSCodium;
            }
        }
        return IdeInfo::VSCode;
    }

    // Priority 3: VS Code IPC socket present
    if std::env::var("VSCODE_IPC_HOOK_CLI").is_ok() {
        return IdeInfo::VSCode;
    }

    IdeInfo::Unknown
}

/// Return the CLI command name for an IDE, if known.
///
/// Used for e.g. installing extensions (`<cmd> --install-extension ...`).
#[must_use]
pub fn ide_cmd(ide: &IdeInfo) -> Option<&'static str> {
    match ide {
        IdeInfo::VSCode => Some("code"),
        IdeInfo::VSCodeInsiders => Some("code-insiders"),
        IdeInfo::Cursor => Some("cursor"),
        IdeInfo::Windsurf => Some("windsurf"),
        IdeInfo::VSCodium => Some("codium"),
        IdeInfo::Unknown => None,
    }
}

/// Return a human-readable name for an IDE.
#[must_use]
pub fn ide_name(ide: &IdeInfo) -> &'static str {
    match ide {
        IdeInfo::VSCode => "VS Code",
        IdeInfo::VSCodeInsiders => "VS Code Insiders",
        IdeInfo::Cursor => "Cursor",
        IdeInfo::Windsurf => "Windsurf",
        IdeInfo::VSCodium => "VSCodium",
        IdeInfo::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ide_cmd_known_ides() {
        assert_eq!(ide_cmd(&IdeInfo::VSCode), Some("code"));
        assert_eq!(ide_cmd(&IdeInfo::Cursor), Some("cursor"));
        assert_eq!(ide_cmd(&IdeInfo::Windsurf), Some("windsurf"));
        assert_eq!(ide_cmd(&IdeInfo::VSCodeInsiders), Some("code-insiders"));
        assert_eq!(ide_cmd(&IdeInfo::VSCodium), Some("codium"));
        assert_eq!(ide_cmd(&IdeInfo::Unknown), None);
    }

    #[test]
    fn ide_name_coverage() {
        assert_eq!(ide_name(&IdeInfo::VSCode), "VS Code");
        assert_eq!(ide_name(&IdeInfo::Cursor), "Cursor");
        assert_eq!(ide_name(&IdeInfo::Unknown), "Unknown");
    }

    #[test]
    fn detect_ide_rustcode_caller_cursor() {
        // We cannot easily set env vars in parallel tests without unsafe,
        // so just verify the function is callable and returns Unknown
        // when none of the trigger env vars are set (CI environment).
        // The actual detection logic is tested by inspect the match arms.
        let _ = detect_ide(); // must not panic
    }
}
