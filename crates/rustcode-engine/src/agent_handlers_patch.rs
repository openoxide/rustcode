use super::{AgentOptions, AgentState, CommandContext, Engine, ExecutionError, PathOperation};
use crate::agent_handlers_fs::diff_output;

impl Engine {
    /// Apply a unified diff patch to workspace files.
    ///
    /// The patch text follows standard unified diff format:
    /// ```text
    /// --- a/path/to/file
    /// +++ b/path/to/file
    /// @@ -start,count +start,count @@
    ///  context line
    /// -removed line
    /// +added line
    /// ```
    ///
    /// Multiple files can be patched in a single call.
    pub(crate) async fn agent_tool_apply_patch(
        &self,
        patch_text: &str,
        context: &CommandContext,
        options: &AgentOptions,
        state: &mut AgentState,
    ) -> Result<String, ExecutionError> {
        // Auto-snapshot before the first mutation this session
        self.auto_snapshot_before_mutation(state, context).await;
        let mut applied = Vec::new();
        let mut rendered_diffs = Vec::new();
        let patches = parse_unified_diff(patch_text)?;
        if !patches.is_empty() {
            for patch in &patches {
                let resolved =
                    self.resolve_workspace_path(context, &patch.path, PathOperation::Edit)?;
                if !patch.is_new_file && !state.read_paths.contains(&resolved) {
                    return Err(ExecutionError::Dispatch(format!(
                        "apply_patch requires reading {} first",
                        patch.path
                    )));
                }

                let original = if patch.is_new_file {
                    String::new()
                } else {
                    self.fs
                        .read_to_string_limited(&resolved, options.max_read_bytes)
                        .await
                        .map_err(|err| ExecutionError::Executor(err.to_string()))?
                };

                let result = apply_hunks(&original, &patch.hunks)?;
                self.fs
                    .write_string(&resolved, &result)
                    .await
                    .map_err(|err| ExecutionError::Executor(err.to_string()))?;
                let diff = diff_output(&original, &result, 40);
                if !diff.is_empty() {
                    rendered_diffs.push((patch.path.clone(), diff));
                }

                // Track as read so subsequent patches/edits can operate
                state.read_paths.insert(resolved);
                applied.push(patch.path.clone());
            }
        } else {
            let codex_ops = parse_codex_patch(patch_text)?;
            if codex_ops.is_empty() {
                return Err(ExecutionError::Dispatch(
                    "no valid patches found in patch text".to_string(),
                ));
            }
            for op in &codex_ops {
                match op {
                    CodexPatchOp::Update { path, hunks } => {
                        let resolved =
                            self.resolve_workspace_path(context, path, PathOperation::Edit)?;
                        if !state.read_paths.contains(&resolved) {
                            return Err(ExecutionError::Dispatch(format!(
                                "apply_patch requires reading {path} first"
                            )));
                        }
                        let original = self
                            .fs
                            .read_to_string_limited(&resolved, options.max_read_bytes)
                            .await
                            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
                        let result = apply_codex_hunks(&original, hunks, path)?;
                        self.fs
                            .write_string(&resolved, &result)
                            .await
                            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
                        let diff = diff_output(&original, &result, 40);
                        if !diff.is_empty() {
                            rendered_diffs.push((path.clone(), diff));
                        }
                        state.read_paths.insert(resolved);
                        applied.push(path.clone());
                    }
                    CodexPatchOp::Add { path, lines } => {
                        let resolved =
                            self.resolve_workspace_path(context, path, PathOperation::Write)?;
                        let result = lines.join("\n");
                        self.fs
                            .write_string(&resolved, &result)
                            .await
                            .map_err(|err| ExecutionError::Executor(err.to_string()))?;
                        let diff = diff_output("", &result, 40);
                        if !diff.is_empty() {
                            rendered_diffs.push((path.clone(), diff));
                        }
                        state.read_paths.insert(resolved);
                        applied.push(path.clone());
                    }
                    CodexPatchOp::Delete { path } => {
                        return Err(ExecutionError::Dispatch(format!(
                            "apply_patch delete-file block is not supported yet: {path}"
                        )));
                    }
                }
            }
        }

        let base_msg = format!(
            "applied {} patch(es) to: {}",
            applied.len(),
            applied.join(", ")
        );
        if rendered_diffs.is_empty() {
            return Ok(base_msg);
        }

        let mut out = base_msg;
        for (path, diff) in rendered_diffs {
            out.push_str(&format!("\nfile: {path}\n{diff}"));
        }
        Ok(out)
    }
}

/// A parsed patch for a single file.
struct FilePatch {
    path: String,
    is_new_file: bool,
    hunks: Vec<Hunk>,
}

/// A single hunk within a unified diff.
struct Hunk {
    /// 1-based start line in the original file.
    old_start: usize,
    lines: Vec<HunkLine>,
}

enum HunkLine {
    Context(String),
    Remove(()),
    Add(String),
}

enum CodexPatchOp {
    Update { path: String, hunks: Vec<CodexHunk> },
    Add { path: String, lines: Vec<String> },
    Delete { path: String },
}

type CodexHunk = Vec<CodexHunkLine>;

enum CodexHunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Parse a unified diff into per-file patches.
fn parse_unified_diff(text: &str) -> Result<Vec<FilePatch>, ExecutionError> {
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

fn parse_codex_patch(text: &str) -> Result<Vec<CodexPatchOp>, ExecutionError> {
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

fn parse_codex_update_hunks(lines: &[&str]) -> Vec<CodexHunk> {
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

fn apply_codex_hunks(
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

/// Parse a single hunk starting with @@ -`old_start,old_count` +`new_start,new_count` @@
fn parse_hunk(lines: &[&str]) -> Result<(Hunk, usize), ExecutionError> {
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
fn normalize_diff_path(path: &str) -> String {
    if path.starts_with("a/") || path.starts_with("b/") {
        path[2..].to_string()
    } else {
        path.to_string()
    }
}

/// Apply hunks to the original content, producing the patched output.
fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, ExecutionError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_patch() {
        let patch = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"world\");
 }
";
        let patches = parse_unified_diff(patch).unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].path, "src/main.rs");
        assert!(!patches[0].is_new_file);
        assert_eq!(patches[0].hunks.len(), 1);
    }

    #[test]
    fn apply_simple_patch() {
        let original = "fn main() {\n    println!(\"hello\");\n}\n";
        let patch = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"world\");
 }
";
        let patches = parse_unified_diff(patch).unwrap();
        let result = apply_hunks(original, &patches[0].hunks).unwrap();
        assert_eq!(result, "fn main() {\n    println!(\"world\");\n}\n");
    }

    #[test]
    fn parse_new_file_patch() {
        let patch = "\
--- /dev/null
+++ b/new_file.txt
@@ -0,0 +1,2 @@
+line one
+line two
";
        let patches = parse_unified_diff(patch).unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].is_new_file);
        assert_eq!(patches[0].path, "new_file.txt");
    }

    #[test]
    fn apply_new_file_patch() {
        let original = "";
        let patch = "\
--- /dev/null
+++ b/new_file.txt
@@ -0,0 +1,2 @@
+line one
+line two
";
        let patches = parse_unified_diff(patch).unwrap();
        let result = apply_hunks(original, &patches[0].hunks).unwrap();
        assert_eq!(result, "line one\nline two");
    }

    #[test]
    fn normalize_diff_path_strips_prefix() {
        assert_eq!(normalize_diff_path("a/src/main.rs"), "src/main.rs");
        assert_eq!(normalize_diff_path("b/src/main.rs"), "src/main.rs");
        assert_eq!(normalize_diff_path("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn parse_codex_update_and_add_patch() {
        let patch = "\
*** Begin Patch
*** Update File: src/main.rs
@@
-fn main() {
-    println!(\"hello\");
+fn main() {
+    println!(\"world\");
 }
*** Add File: notes.txt
+line one
+line two
*** End Patch
";

        let ops = parse_codex_patch(patch).unwrap();
        assert_eq!(ops.len(), 2);
        match &ops[0] {
            CodexPatchOp::Update { path, hunks } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(hunks.len(), 1);
            }
            _ => panic!("expected update op"),
        }
        match &ops[1] {
            CodexPatchOp::Add { path, lines } => {
                assert_eq!(path, "notes.txt");
                assert_eq!(lines, &vec!["line one".to_string(), "line two".to_string()]);
            }
            _ => panic!("expected add op"),
        }
    }

    #[test]
    fn apply_codex_hunks_replaces_expected_block() {
        let original = "fn main() {\n    println!(\"hello\");\n}\n";
        let patch = "\
*** Begin Patch
*** Update File: src/main.rs
@@
 fn main() {
-    println!(\"hello\");
+    println!(\"world\");
 }
*** End Patch
";
        let ops = parse_codex_patch(patch).unwrap();
        let CodexPatchOp::Update { path, hunks } = &ops[0] else {
            panic!("expected update op");
        };
        let result = apply_codex_hunks(original, hunks, path).unwrap();
        assert_eq!(result, "fn main() {\n    println!(\"world\");\n}\n");
    }

    #[test]
    fn apply_codex_hunks_errors_when_context_not_found() {
        let original = "a\nb\nc\n";
        let patch = "\
*** Begin Patch
*** Update File: sample.txt
@@
 x
-y
+z
*** End Patch
";
        let ops = parse_codex_patch(patch).unwrap();
        let CodexPatchOp::Update { path, hunks } = &ops[0] else {
            panic!("expected update op");
        };
        let err = apply_codex_hunks(original, hunks, path).unwrap_err();
        assert!(format!("{err}").contains("could not be applied"));
    }
}
