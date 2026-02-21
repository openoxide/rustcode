use rustcode_core::error::ExecutionError;

/// A parsed patch for a single file.
pub(super) struct FilePatch {
    pub path: String,
    pub is_new_file: bool,
    pub hunks: Vec<Hunk>,
}

/// A single hunk within a unified diff.
pub(super) struct Hunk {
    /// 1-based start line in the original file.
    pub old_start: usize,
    pub lines: Vec<HunkLine>,
}

pub(super) enum HunkLine {
    Context(String),
    Remove(()),
    Add(String),
}

/// Parse a unified diff into per-file patches.
pub(super) fn parse_unified_diff(text: &str) -> Result<Vec<FilePatch>, ExecutionError> {
    let mut patches = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Find --- header
        if !lines[i].starts_with("--- ") {
            i += 1;
            continue;
        }
        if i + 1 >= lines.len() || !lines[i + 1].starts_with("+++ ") {
            i += 1;
            continue;
        }

        let old_path = lines[i].trim_start_matches("--- ").trim();
        let new_path = lines[i + 1].trim_start_matches("+++ ").trim();
        i += 2;

        let is_new_file = old_path == "/dev/null" || old_path == "a//dev/null";
        let path = normalize_diff_path(new_path);

        let mut hunks = Vec::new();
        while i < lines.len() && lines[i].starts_with("@@ ") {
            let (hunk, consumed) = parse_hunk(&lines[i..])?;
            hunks.push(hunk);
            i += consumed;
        }

        if !hunks.is_empty() {
            patches.push(FilePatch {
                path,
                is_new_file,
                hunks,
            });
        }
    }

    Ok(patches)
}

/// Parse a single hunk starting with @@ -`old_start,old_count` +`new_start,new_count` @@
pub(super) fn parse_hunk(lines: &[&str]) -> Result<(Hunk, usize), ExecutionError> {
    let header = lines[0];
    let old_start = parse_hunk_header_old_start(header)?;

    let mut hunk_lines = Vec::new();
    let mut consumed = 1;

    for line in &lines[1..] {
        if line.starts_with("@@ ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            break;
        }
        consumed += 1;

        if line.strip_prefix('-').is_some() {
            hunk_lines.push(HunkLine::Remove(()));
        } else if let Some(rest) = line.strip_prefix('+') {
            hunk_lines.push(HunkLine::Add(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix(' ') {
            hunk_lines.push(HunkLine::Context(rest.to_string()));
        } else if line.starts_with('\\') {
            // "\ No newline at end of file" — skip
        } else {
            // Treat bare lines as context
            hunk_lines.push(HunkLine::Context(line.to_string()));
        }
    }

    Ok((
        Hunk {
            old_start,
            lines: hunk_lines,
        },
        consumed,
    ))
}

/// Parse the old start line from a hunk header like "@@ -10,5 +12,7 @@".
fn parse_hunk_header_old_start(header: &str) -> Result<usize, ExecutionError> {
    let after_at = header
        .strip_prefix("@@ -")
        .ok_or_else(|| ExecutionError::Dispatch("invalid hunk header".to_string()))?;
    let num_str = after_at.split([',', ' ']).next().unwrap_or("1");
    num_str
        .parse::<usize>()
        .map_err(|_| ExecutionError::Dispatch(format!("invalid hunk start line: {num_str}")))
}

/// Normalize a diff path like "a/src/main.rs" or "b/src/main.rs" to "src/main.rs".
pub(super) fn normalize_diff_path(path: &str) -> String {
    if path.starts_with("a/") || path.starts_with("b/") {
        path[2..].to_string()
    } else {
        path.to_string()
    }
}

/// Apply hunks to the original content, producing the patched output.
pub(super) fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, ExecutionError> {
    let original_lines: Vec<&str> = original.lines().collect();
    let mut result = Vec::new();
    let mut pos = 0; // current position in original_lines (0-indexed)

    for hunk in hunks {
        let hunk_start = if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start - 1
        };

        // Copy lines before this hunk
        while pos < hunk_start && pos < original_lines.len() {
            result.push(original_lines[pos].to_string());
            pos += 1;
        }

        // Apply hunk lines
        for hl in &hunk.lines {
            match hl {
                HunkLine::Context(line) => {
                    if pos < original_lines.len() {
                        // Use original line to preserve whitespace exactly
                        result.push(original_lines[pos].to_string());
                        pos += 1;
                    } else {
                        result.push(line.clone());
                    }
                }
                HunkLine::Remove(()) => {
                    // Skip this line from original
                    pos += 1;
                }
                HunkLine::Add(line) => {
                    result.push(line.clone());
                }
            }
        }
    }

    // Copy remaining lines
    while pos < original_lines.len() {
        result.push(original_lines[pos].to_string());
        pos += 1;
    }

    let mut output = result.join("\n");
    // Preserve trailing newline if original had one
    if original.ends_with('\n') && !output.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}
