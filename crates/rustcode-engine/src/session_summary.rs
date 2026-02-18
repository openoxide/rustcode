// ── Session summary ───────────────────────────────────────────────────
//
// Computes git diff statistics for a session to show what changed during
// an agent run — files changed, lines added, lines deleted.

use std::path::Path;
use std::process::Command;

/// Summary of file changes made during a session.
#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    /// Number of files changed.
    pub files_changed: u32,
    /// Total lines added.
    pub additions: u32,
    /// Total lines deleted.
    pub deletions: u32,
}

impl SessionSummary {
    /// Format the summary as a human-readable string.
    pub fn to_display(&self) -> String {
        if self.files_changed == 0 {
            return "No files changed".to_string();
        }
        format!(
            "{} file{} changed, {} insertion{}, {} deletion{}",
            self.files_changed,
            if self.files_changed == 1 { "" } else { "s" },
            self.additions,
            if self.additions == 1 { "" } else { "s" },
            self.deletions,
            if self.deletions == 1 { "" } else { "s" },
        )
    }

    /// Return true if any files were changed.
    pub fn has_changes(&self) -> bool {
        self.files_changed > 0
    }
}

/// Compute diff statistics for the working directory using `git diff --stat`.
///
/// Returns a `SessionSummary` with counts of files changed, insertions, and deletions.
/// If the workspace is not a git repo or git is unavailable, returns an empty summary.
pub fn compute_diff_stats(workspace_root: &Path) -> SessionSummary {
    let output = Command::new("git")
        .args(["diff", "--stat", "--numstat", "HEAD"])
        .current_dir(workspace_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return SessionSummary::default(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_numstat(&stdout)
}

/// Compute diff statistics against a specific git reference (commit, branch, etc.).
///
/// Useful for comparing against a snapshot taken at the start of an agent run.
pub fn compute_diff_since(workspace_root: &Path, since_ref: &str) -> SessionSummary {
    let output = Command::new("git")
        .args(["diff", "--numstat", since_ref])
        .current_dir(workspace_root)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return SessionSummary::default(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_numstat(&stdout)
}

/// Get the current git HEAD reference for later diffing.
///
/// Returns `None` if git is unavailable or the directory is not a repo.
pub fn current_git_ref(workspace_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()?;

    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }
    None
}

/// Parse `git diff --numstat` output into a `SessionSummary`.
///
/// Each line is formatted as: `<additions>\t<deletions>\t<filename>`
/// Binary files show as `-\t-\t<filename>`.
fn parse_numstat(output: &str) -> SessionSummary {
    let mut summary = SessionSummary::default();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            summary.files_changed += 1;
            if let Ok(adds) = parts[0].parse::<u32>() {
                summary.additions += adds;
            }
            if let Ok(dels) = parts[1].parse::<u32>() {
                summary.deletions += dels;
            }
        }
    }

    summary
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_numstat_basic() {
        let output = "10\t5\tsrc/main.rs\n3\t0\tsrc/lib.rs\n";
        let summary = parse_numstat(output);
        assert_eq!(summary.files_changed, 2);
        assert_eq!(summary.additions, 13);
        assert_eq!(summary.deletions, 5);
    }

    #[test]
    fn parse_numstat_binary_files() {
        let output = "-\t-\timage.png\n5\t2\tsrc/lib.rs\n";
        let summary = parse_numstat(output);
        assert_eq!(summary.files_changed, 2);
        assert_eq!(summary.additions, 5); // binary counted as file but 0 lines
        assert_eq!(summary.deletions, 2);
    }

    #[test]
    fn parse_numstat_empty() {
        let summary = parse_numstat("");
        assert_eq!(summary.files_changed, 0);
        assert_eq!(summary.additions, 0);
        assert_eq!(summary.deletions, 0);
    }

    #[test]
    fn display_no_changes() {
        let summary = SessionSummary::default();
        assert_eq!(summary.to_display(), "No files changed");
        assert!(!summary.has_changes());
    }

    #[test]
    fn display_single_file() {
        let summary = SessionSummary {
            files_changed: 1,
            additions: 1,
            deletions: 0,
        };
        assert_eq!(summary.to_display(), "1 file changed, 1 insertion, 0 deletions");
        assert!(summary.has_changes());
    }

    #[test]
    fn display_multiple_files() {
        let summary = SessionSummary {
            files_changed: 3,
            additions: 42,
            deletions: 7,
        };
        assert_eq!(summary.to_display(), "3 files changed, 42 insertions, 7 deletions");
    }

    #[test]
    fn compute_diff_stats_handles_non_repo() {
        let summary = compute_diff_stats(Path::new("/tmp/not-a-git-repo-12345"));
        assert_eq!(summary.files_changed, 0);
    }

    #[test]
    fn current_git_ref_handles_non_repo() {
        let result = current_git_ref(Path::new("/tmp/not-a-git-repo-12345"));
        assert!(result.is_none());
    }
}
