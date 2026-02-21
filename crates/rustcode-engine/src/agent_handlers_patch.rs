mod parse_codex;
mod parse_unified;

use super::{AgentOptions, AgentState, CommandContext, Engine, ExecutionError, PathOperation};
use crate::agent_handlers_fs::diff_output;
use parse_codex::{apply_codex_hunks, parse_codex_patch, CodexPatchOp};
use parse_unified::{apply_hunks, parse_unified_diff};

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
        if patches.is_empty() {
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
        } else {
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

#[cfg(test)]
mod tests {
    use super::parse_codex::{apply_codex_hunks, parse_codex_patch, CodexPatchOp};
    use super::parse_unified::{apply_hunks, normalize_diff_path, parse_unified_diff};

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
