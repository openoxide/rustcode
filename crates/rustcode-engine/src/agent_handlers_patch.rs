use super::{AgentOptions, AgentState, CommandContext, Engine, ExecutionError, PathOperation};

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
        let patches = parse_unified_diff(patch_text)?;
        if patches.is_empty() {
            return Err(ExecutionError::Dispatch(
                "no valid patches found in patch text".to_string(),
            ));
        }

        let mut applied = Vec::new();
        for patch in &patches {
            let resolved =
                self.resolve_workspace_path(context, &patch.path, PathOperation::Edit)?;

            // For new files (--- /dev/null), skip the read check
            let is_new_file = patch.is_new_file;
            if !is_new_file && !state.read_paths.contains(&resolved) {
                return Err(ExecutionError::Dispatch(format!(
                    "apply_patch requires reading {} first",
                    patch.path
                )));
            }

            let original = if is_new_file {
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

            // Track as read so subsequent patches/edits can operate
            state.read_paths.insert(resolved);
            applied.push(patch.path.clone());
        }

        Ok(format!(
            "applied {} patch(es) to: {}",
            applied.len(),
            applied.join(", ")
        ))
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
}
