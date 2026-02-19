// ── CLI worktree commands ─────────────────────────────────────────────────
//
// Thin wrappers that invoke git directly (no engine required).

use anyhow::{bail, Result};

use crate::cli::WorktreeCommand;

/// Handle the `rustcode worktree` subcommand family.
pub fn handle_worktree_command(command: &WorktreeCommand, json: bool) -> Result<()> {
    match command {
        WorktreeCommand::Create { name, branch } => {
            create(name.as_deref(), branch.as_deref(), json)
        }
        WorktreeCommand::List => list(json),
        WorktreeCommand::Remove { path } => remove(path, json),
        WorktreeCommand::Reset { path } => reset(path, json),
    }
}

fn create(name: Option<&str>, branch: Option<&str>, json: bool) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let wt_name = name.map_or_else(
        || {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("wt-{ts}")
        },
        str::to_string,
    );
    let wt_branch = branch.map_or_else(|| wt_name.clone(), str::to_string);

    // Resolve path as sibling of CWD
    let cwd = std::env::current_dir()?;
    let parent = cwd
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current directory has no parent"))?;
    let wt_path = parent.join(&wt_name);
    let wt_path_str = wt_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("worktree path is not valid UTF-8"))?
        .to_string();

    let status = std::process::Command::new("git")
        .args(["worktree", "add", "-b", &wt_branch, &wt_path_str])
        .status()?;

    if !status.success() {
        bail!(
            "git worktree add failed with exit code {}",
            status.code().unwrap_or(-1)
        );
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "name": wt_name,
                "branch": wt_branch,
                "path": wt_path_str,
            })
        );
    } else {
        println!("worktree created");
        println!("name:   {wt_name}");
        println!("branch: {wt_branch}");
        println!("path:   {wt_path_str}");
    }
    Ok(())
}

fn list(json: bool) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()?;

    if !output.status.success() {
        bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let entries = parse_porcelain(&raw);

    if json {
        let arr: Vec<_> = entries
            .iter()
            .map(|(path, branch, bare)| {
                serde_json::json!({ "path": path, "branch": branch, "bare": bare })
            })
            .collect();
        println!("{}", serde_json::json!(arr));
    } else if entries.is_empty() {
        println!("no worktrees");
    } else {
        for (path, branch, bare) in &entries {
            println!(
                "  {}  [{}]{}",
                path,
                branch,
                if *bare { "  (bare)" } else { "" }
            );
        }
    }
    Ok(())
}

fn remove(path: &str, json: bool) -> Result<()> {
    let status = std::process::Command::new("git")
        .args(["worktree", "remove", "--force", path])
        .status()?;

    if !status.success() {
        bail!(
            "git worktree remove failed with exit code {}",
            status.code().unwrap_or(-1)
        );
    }

    if json {
        println!("{}", serde_json::json!({ "removed": path }));
    } else {
        println!("worktree removed: {path}");
    }
    Ok(())
}

fn reset(path: &str, json: bool) -> Result<()> {
    for (args, label) in [
        (vec!["-C", path, "fetch", "--prune"], "fetch"),
        (vec!["-C", path, "reset", "--hard", "@{u}"], "reset"),
        (vec!["-C", path, "clean", "-ffdx"], "clean"),
    ] {
        let status = std::process::Command::new("git").args(&args).status()?;
        if !status.success() {
            bail!(
                "git {} failed with exit code {}",
                label,
                status.code().unwrap_or(-1)
            );
        }
    }

    if json {
        println!("{}", serde_json::json!({ "reset": path }));
    } else {
        println!("worktree reset to upstream: {path}");
    }
    Ok(())
}

/// Parse `git worktree list --porcelain` → `Vec<(path, branch, bare)>`.
fn parse_porcelain(raw: &str) -> Vec<(String, String, bool)> {
    let mut result = Vec::new();
    let mut path = String::new();
    let mut branch = String::new();
    let mut bare = false;

    for line in raw.lines() {
        if line.is_empty() {
            if !path.is_empty() {
                result.push((
                    std::mem::take(&mut path),
                    if branch.is_empty() {
                        "detached".to_string()
                    } else {
                        std::mem::take(&mut branch)
                    },
                    bare,
                ));
                bare = false;
            }
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = b.to_string();
        } else if line == "bare" {
            bare = true;
        }
    }
    if !path.is_empty() {
        result.push((
            path,
            if branch.is_empty() {
                "detached".to_string()
            } else {
                branch
            },
            bare,
        ));
    }
    result
}
