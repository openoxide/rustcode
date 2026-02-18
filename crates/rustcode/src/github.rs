use crate::cli::{GitHubCommand, PrCommand};
use crate::utils::write_stdout_line;
use anyhow::{Context, Result};
use std::path::Path;

pub fn handle_github_command(command: &GitHubCommand, json_output: bool) -> Result<()> {
    match command {
        GitHubCommand::Repo => {
            let cwd = std::env::current_dir().context("failed to get current dir")?;
            ensure_git_repo(&cwd)?;
            let url = git_origin_url(&cwd)?;
            let (owner, repo) =
                parse_github_remote(&url).context("failed to parse github remote from origin")?;
            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "github.repo",
                    "owner": owner,
                    "repo": repo,
                    "url": url,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("owner={owner}"))?;
                write_stdout_line(&format!("repo={repo}"))?;
                write_stdout_line(&format!("url={url}"))?;
            }
        }
        GitHubCommand::Status => {
            let cwd = std::env::current_dir().context("failed to get current dir")?;
            let has_git = ensure_git_repo(&cwd).is_ok();
            let has_gh = ensure_gh_installed().is_ok();
            let gh_auth = if has_gh {
                gh_auth_status(&cwd).unwrap_or(false)
            } else {
                false
            };
            let gh_ver = if has_gh { gh_version().ok() } else { None };

            if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "github.status",
                    "git": has_git,
                    "gh": has_gh,
                    "gh_auth": gh_auth,
                    "gh_version": gh_ver,
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!(
                    "git:    {}",
                    if has_git { "found" } else { "not found" }
                ))?;
                write_stdout_line(&format!(
                    "gh:     {}",
                    if has_gh { "found" } else { "not found" }
                ))?;
                if let Some(v) = gh_ver {
                    write_stdout_line(&format!("gh-ver: {v}"))?;
                }
                write_stdout_line(&format!(
                    "gh-auth: {}",
                    if gh_auth {
                        "authorized"
                    } else {
                        "not authorized"
                    }
                ))?;
            }
        }
    }
    Ok(())
}

pub fn handle_pr_command(command: PrCommand, json_output: bool) -> Result<()> {
    match command {
        PrCommand::Checkout {
            number,
            branch,
            force,
            tui,
            ..
        } => {
            ensure_gh_installed()?;
            let mut cmd = std::process::Command::new("gh");
            cmd.args(["pr", "checkout", &number.to_string()]);
            if let Some(b) = branch {
                cmd.args(["--branch", &b]);
            }
            if force {
                cmd.arg("--force");
            }

            let status = cmd.status().context("failed to execute gh pr checkout")?;
            if !status.success() {
                anyhow::bail!("gh pr checkout failed");
            }

            if tui {
                // handle TUI launch logic if needed, or just print instructions
                write_stdout_line("PR checked out. Launching TUI...")?;
            } else if json_output {
                let payload = serde_json::json!({
                    "schema_version": 1,
                    "command": "pr.checkout",
                    "number": number,
                    "status": "success",
                });
                write_stdout_line(&serde_json::to_string(&payload)?)?;
            } else {
                write_stdout_line(&format!("Checked out PR #{number}"))?;
            }
        }
        PrCommand::Create {
            title,
            body,
            base,
            head,
            draft,
            fill,
        } => {
            ensure_gh_installed()?;
            let mut cmd = std::process::Command::new("gh");
            cmd.args(["pr", "create"]);
            if let Some(t) = title {
                cmd.args(["--title", &t]);
            }
            if let Some(b) = body {
                cmd.args(["--body", &b]);
            }
            if let Some(b) = base {
                cmd.args(["--base", &b]);
            }
            if let Some(h) = head {
                cmd.args(["--head", &h]);
            }
            if draft {
                cmd.arg("--draft");
            }
            if fill {
                cmd.arg("--fill");
            }

            let status = cmd.status().context("failed to execute gh pr create")?;
            if !status.success() {
                anyhow::bail!("gh pr create failed");
            }
        }
    }
    Ok(())
}

pub fn ensure_git_repo(cwd: &Path) -> Result<()> {
    if !cwd.join(".git").exists() {
        anyhow::bail!("not a git repository (no .git directory found)");
    }
    Ok(())
}

pub fn git_origin_url(cwd: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(["remote", "get-url", "origin"])
        .output()
        .context("failed to execute git remote get-url origin")?;
    if !output.status.success() {
        anyhow::bail!("failed to get git origin url");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();

    // Handle https remotes
    if let Some(pos) = trimmed.to_ascii_lowercase().find("github.com/") {
        let rest = &trimmed[pos + "github.com/".len()..];
        let mut parts = rest.splitn(3, '/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        let repo = repo
            .split_once('?')
            .map_or(repo, |(value, _)| value)
            .split_once('#')
            .map_or(repo, |(value, _)| value)
            .strip_suffix(".git")
            .unwrap_or(repo);
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    // git@github.com:owner/repo(.git)
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        let (owner, repo) = rest.split_once('/')?;
        let repo = repo.strip_suffix(".git").unwrap_or(repo);
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    // ssh://git@github.com/owner/repo(.git)
    if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        let (owner, repo) = rest.split_once('/')?;
        let repo = repo.strip_suffix(".git").unwrap_or(repo);
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string()));
    }

    None
}

pub fn ensure_gh_installed() -> Result<()> {
    let output = std::process::Command::new("gh")
        .arg("--version")
        .output()
        .context("failed to execute gh")?;
    if !output.status.success() {
        anyhow::bail!("gh is not available (install GitHub CLI)");
    }
    Ok(())
}

pub fn gh_version() -> Result<String> {
    let output = std::process::Command::new("gh")
        .arg("--version")
        .output()
        .context("failed to execute gh")?;
    if !output.status.success() {
        anyhow::bail!("gh --version failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string())
}

pub fn gh_auth_status(cwd: &Path) -> Result<bool> {
    let output = std::process::Command::new("gh")
        .current_dir(cwd)
        .args(["auth", "status"])
        .output()
        .context("failed to execute gh auth status")?;
    Ok(output.status.success())
}
