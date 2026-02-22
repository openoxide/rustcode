use std::path::Path;

pub(super) fn build_environment_block(model: &str, workspace_root: &Path, is_git_repo: bool) -> String {
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unknown"
    };

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    let today = format_today();

    format!(
        "<environment>\n\
         Model: {model}\n\
         Working directory: {workspace}\n\
         Git repository: {git}\n\
         Platform: {platform}\n\
         Shell: {shell}\n\
         Today's date: {today}\n\
         </environment>",
        workspace = workspace_root.display(),
        git = if is_git_repo { "yes" } else { "no" },
    )
}

fn format_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days = (secs / 86400) as i64;
    let (year, month, day) = days_to_ymd(days);

    let weekday = match (days % 7 + 4) % 7 {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    };

    format!("{year}-{month:02}-{day:02} ({weekday})")
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub(super) fn is_git_repo(workspace_root: &Path) -> bool {
    workspace_root.join(".git").exists()
}

#[cfg(test)]
pub(super) fn format_today_for_test() -> String {
    format_today()
}

#[cfg(test)]
pub(super) fn days_to_ymd_for_test(days: i64) -> (i64, u32, u32) {
    days_to_ymd(days)
}
