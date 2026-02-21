use std::path::PathBuf;

use super::super::GitStat;
use crate::InteractiveMsg;

/// Spawn a background task to compute git diff stats, sending the result
/// back via the TUI message channel.
///
/// Uses `spawn_blocking` to avoid blocking the async event loop.
pub(super) fn refresh_git_stat_async(
    workspace_root: PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<InteractiveMsg>,
) {
    tokio::task::spawn_blocking(move || {
        if let Some(stat) = compute_git_stat(&workspace_root) {
            let _ = tx.send(InteractiveMsg::GitStatUpdate {
                files: stat.files,
                insertions: stat.insertions,
                deletions: stat.deletions,
            });
        }
    });
}

/// Run `git diff --shortstat HEAD` synchronously and parse the output.
fn compute_git_stat(workspace_root: &std::path::Path) -> Option<GitStat> {
    let output = std::process::Command::new("git")
        .args(["diff", "--shortstat", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    parse_git_shortstat(text.trim())
}

/// Parse the output of `git diff --shortstat HEAD` into a `GitStat`.
///
/// Example inputs:
/// - `"1 file changed, 3 insertions(+), 1 deletion(-)"`
/// - `"15 files changed, 406 insertions(+), 269 deletions(-)"`
/// - `"1 file changed, 1 insertion(+)"`
fn parse_git_shortstat(text: &str) -> Option<GitStat> {
    if text.is_empty() {
        return None;
    }
    let files: u32 = text
        .split_once(" file")
        .and_then(|(n, _)| n.trim().parse().ok())?;

    let insertions: u32 = text
        .find("insertion")
        .and_then(|pos| {
            text[..pos]
                .trim()
                .rsplit_once(|c: char| !c.is_ascii_digit())
                .and_then(|(_, n)| n.parse().ok())
        })
        .unwrap_or(0);

    let deletions: u32 = text
        .find("deletion")
        .and_then(|pos| {
            text[..pos]
                .trim()
                .rsplit_once(|c: char| !c.is_ascii_digit())
                .and_then(|(_, n)| n.parse().ok())
        })
        .unwrap_or(0);

    Some(GitStat {
        files,
        insertions,
        deletions,
    })
}
