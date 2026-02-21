use rustcode_core::error::ExecutionError;

pub(super) enum CodexPatchOp {
    Update { path: String, hunks: Vec<CodexHunk> },
    Add { path: String, lines: Vec<String> },
    Delete { path: String },
}

pub(super) type CodexHunk = Vec<CodexHunkLine>;

pub(super) enum CodexHunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

pub(super) fn parse_codex_patch(text: &str) -> Result<Vec<CodexPatchOp>, ExecutionError> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(begin_idx) = lines
        .iter()
        .position(|line| line.trim() == "*** Begin Patch")
    else {
        return Ok(Vec::new());
    };

    let mut ops = Vec::new();
    let mut i = begin_idx + 1;
    while i < lines.len() {
        let line = lines[i].trim_end();
        if line == "*** End Patch" {
            break;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            i += 1;
            let start = i;
            while i < lines.len() && !lines[i].starts_with("*** ") {
                i += 1;
            }
            let body = &lines[start..i];
            let hunks = parse_codex_update_hunks(body);
            if hunks.is_empty() {
                return Err(ExecutionError::Dispatch(format!(
                    "apply_patch update block has no valid hunks for {path}"
                )));
            }
            ops.push(CodexPatchOp::Update { path, hunks });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = path.trim().to_string();
            i += 1;
            let start = i;
            while i < lines.len() && !lines[i].starts_with("*** ") {
                i += 1;
            }
            let body = &lines[start..i];
            let added_lines = body
                .iter()
                .map(|raw| raw.strip_prefix('+').unwrap_or(raw).to_string())
                .collect::<Vec<_>>();
            ops.push(CodexPatchOp::Add {
                path,
                lines: added_lines,
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(CodexPatchOp::Delete {
                path: path.trim().to_string(),
            });
            i += 1;
            continue;
        }
        i += 1;
    }
    Ok(ops)
}

pub(super) fn parse_codex_update_hunks(lines: &[&str]) -> Vec<CodexHunk> {
    let mut hunks = Vec::new();
    let mut current = Vec::new();

    for raw in lines {
        if raw.starts_with("@@") {
            if !current.is_empty() {
                hunks.push(current);
                current = Vec::new();
            }
            continue;
        }
        if raw.starts_with("*** End of File") || raw.starts_with("\\ No newline") {
            continue;
        }
        if let Some(rest) = raw.strip_prefix('+') {
            current.push(CodexHunkLine::Add(rest.to_string()));
        } else if let Some(rest) = raw.strip_prefix('-') {
            current.push(CodexHunkLine::Remove(rest.to_string()));
        } else if let Some(rest) = raw.strip_prefix(' ') {
            current.push(CodexHunkLine::Context(rest.to_string()));
        } else {
            current.push(CodexHunkLine::Context((*raw).to_string()));
        }
    }

    if !current.is_empty() {
        hunks.push(current);
    }
    hunks
}

pub(super) fn apply_codex_hunks(
    original: &str,
    hunks: &[CodexHunk],
    path: &str,
) -> Result<String, ExecutionError> {
    let had_trailing_newline = original.ends_with('\n');
    let mut lines: Vec<String> = original.lines().map(ToString::to_string).collect();
    let mut search_from = 0usize;

    for (hunk_idx, hunk) in hunks.iter().enumerate() {
        let mut old_seq = Vec::new();
        let mut new_seq = Vec::new();
        for line in hunk {
            match line {
                CodexHunkLine::Context(text) => {
                    old_seq.push(text.clone());
                    new_seq.push(text.clone());
                }
                CodexHunkLine::Remove(text) => old_seq.push(text.clone()),
                CodexHunkLine::Add(text) => new_seq.push(text.clone()),
            }
        }

        if old_seq.is_empty() {
            return Err(ExecutionError::Dispatch(format!(
                "apply_patch hunk {} for {} has no anchor/context",
                hunk_idx + 1,
                path
            )));
        }

        let start = find_subsequence(&lines, &old_seq, search_from)
            .or_else(|| find_subsequence(&lines, &old_seq, 0))
            .ok_or_else(|| {
                ExecutionError::Dispatch(format!(
                    "apply_patch hunk {} for {} could not be applied (context not found)",
                    hunk_idx + 1,
                    path
                ))
            })?;

        let end = start + old_seq.len();
        lines.splice(start..end, new_seq.into_iter());
        search_from = start.saturating_add(1);
    }

    let mut output = lines.join("\n");
    if had_trailing_newline && !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn find_subsequence(haystack: &[String], needle: &[String], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    if needle.len() > haystack.len() || start > haystack.len().saturating_sub(needle.len()) {
        return None;
    }
    for idx in start..=haystack.len() - needle.len() {
        if haystack[idx..idx + needle.len()] == *needle {
            return Some(idx);
        }
    }
    None
}
