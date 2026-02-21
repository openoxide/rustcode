use super::super::AppState;
use super::super::GitStat;

/// Refresh git diff stats by running `git diff --shortstat HEAD` in the workspace root.
///
/// Non-blocking: if git is unavailable or the directory isn't a repo, the stat is cleared.
pub(super) fn refresh_git_stat(state: &mut AppState) {
    let Ok(output) = std::process::Command::new("git")
        .args(["diff", "--shortstat", "HEAD"])
        .current_dir(&state.defaults.workspace_root)
        .output()
    else {
        state.git_stat = None;
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    state.git_stat = parse_git_shortstat(text.trim());
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
