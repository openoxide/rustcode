/// File search utilities — enumerate workspace files for the file-picker popup.
use std::path::Path;

const MAX_FILES: usize = 2000;
const MAX_DEPTH: usize = 8;

static SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".cargo",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
];

/// Scan `root` recursively and return relative file paths (sorted, limited to `MAX_FILES`).
pub(super) fn scan_workspace_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    scan_dir(root, root, 0, &mut out);
    out.sort_unstable();
    out
}

fn scan_dir(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            break;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden dirs and known noise dirs
        if name_str.starts_with('.') && name_str != ".rustcode.md" && path.is_dir() {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            scan_dir(root, &path, depth + 1, out);
        } else {
            let rel = path.strip_prefix(root).map_or_else(
                |_| path.to_string_lossy().into_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
            out.push(rel);
        }
    }
}

/// Filter entries by fuzzy substring match on the query.
pub(super) fn filter_files(entries: &[String], query: &str) -> Vec<usize> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return (0..entries.len().min(200)).collect();
    }
    let mut out = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        if entry.to_ascii_lowercase().contains(&q) {
            out.push(idx);
            if out.len() >= 200 {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_files_empty_query_returns_first_200() {
        let entries: Vec<String> = (0..300).map(|i| format!("file_{i}.rs")).collect();
        let result = filter_files(&entries, "");
        assert_eq!(result.len(), 200);
        assert_eq!(result[0], 0);
    }

    #[test]
    fn filter_files_matches_substring() {
        let entries = vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "README.md".to_string(),
        ];
        let result = filter_files(&entries, "lib");
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn filter_files_case_insensitive() {
        let entries = vec!["Cargo.toml".to_string(), "cargo.lock".to_string()];
        let result = filter_files(&entries, "CARGO");
        assert_eq!(result.len(), 2);
    }
}
