use std::path::Path;

use super::{
    Block, Borders, Line, Modifier, Paragraph, Rect, Span, Style, ToolApprovalRequest, Wrap,
};
use crate::interactive::theme;

mod preview;
use preview::{render_approval_preview, tilde_path};

pub(super) fn approval_is_command_permission(permission: &str) -> bool {
    matches!(
        permission.to_ascii_lowercase().as_str(),
        "exec" | "bash" | "pty_exec"
    )
}

pub(super) fn approval_option_labels(req: &ToolApprovalRequest) -> Vec<String> {
    let mut options = vec![format!("Approve once [{}]", req.tool)];
    if approval_is_command_permission(&req.permission) {
        options.push("Allow all edits".to_string());
    }
    options.push("Approve all tools (auto-pilot mode)".to_string());
    options.push("Deny".to_string());
    options
}

pub(super) fn approval_options_count(req: &ToolApprovalRequest) -> usize {
    approval_option_labels(req).len()
}

/// Build approval info lines to append to the transcript when a tool approval is pending.
///
/// Shows the action title, reason (if any), and a short content preview (up to 8 lines).
/// `width` is the inner width of the transcript area so diff backgrounds fill the full row.
pub(super) fn render_approval_inline(
    req: &ToolApprovalRequest,
    width: u16,
    workspace_root: &Path,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::raw("")));
    // Show diff preview first, then the approval title/reason below it.
    lines.extend(render_approval_preview(req, width, workspace_root));
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(vec![
        Span::styled(
            "  Approval  ",
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            approval_action_title(req),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines
}

/// Render the approval selector widget in place of the composer.
///
/// Shows ←/→ navigable options inside a yellow-bordered "Approval" block.
/// The `selected` index highlights the active option with a reversed-video indicator.
pub(super) fn render_approval_selector(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    req: &ToolApprovalRequest,
    selected: usize,
) {
    let options = approval_option_labels(req);

    let mut option_lines: Vec<Line<'static>> = Vec::new();
    for (i, opt) in options.into_iter().enumerate() {
        if i == selected {
            option_lines.push(Line::from(Span::styled(
                format!("  ❯ {opt}"),
                Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            )));
        } else {
            option_lines.push(Line::from(Span::styled(
                format!("    {opt}"),
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
    }
    option_lines.push(Line::from(Span::styled(
        "  ↑/↓ navigate   Enter: confirm   Esc: deny",
        Style::default().add_modifier(Modifier::DIM),
    )));

    frame.render_widget(
        Paragraph::new(option_lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        "Approval",
                        Style::default()
                            .fg(theme::WARNING)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::WARNING)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn approval_action_title(req: &ToolApprovalRequest) -> String {
    let perm = req.permission.to_lowercase();
    let raw_path = req
        .arguments
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(&req.pattern);
    // Show full path with ~ replacing $HOME.
    let display_path = tilde_path(raw_path);
    if perm == "write" || perm == "edit" {
        format!("Write / edit:  {display_path}")
    } else if perm == "exec" {
        "Run command".to_string()
    } else {
        req.tool.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_to_text(lines: &[Line<'_>]) -> String {
        let mut out = String::new();
        for line in lines {
            for span in &line.spans {
                out.push_str(span.content.as_ref());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn approval_preview_shows_apply_patch_diff_lines() {
        let req = ToolApprovalRequest {
            tool: "apply_patch".to_string(),
            permission: "write".to_string(),
            pattern: "src/main.rs".to_string(),
            arguments: serde_json::json!({
                "patch_text": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n"
            }),
            reason: String::new(),
        };

        let lines = render_approval_inline(&req, 120, Path::new("."));
        let text = lines_to_text(&lines);
        // @@ hunk header is shown as "… line N" separator
        assert!(text.contains("… line"), "expected separator in text={text}");
        // same style as write/edit: "  NNN - code" and "  NNN + code"
        assert!(text.contains("- old"), "text={text}");
        assert!(text.contains("+ new"), "text={text}");
        assert!(text.contains("src/main.rs"), "text={text}");
    }

    #[test]
    fn approval_preview_shows_multiedit_blocks() {
        let req = ToolApprovalRequest {
            tool: "multiedit".to_string(),
            permission: "write".to_string(),
            pattern: "src/lib.rs".to_string(),
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "edits": [
                    { "old_string": "foo", "new_string": "bar", "replace_all": false }
                ]
            }),
            reason: String::new(),
        };

        let lines = render_approval_inline(&req, 120, Path::new("."));
        let text = lines_to_text(&lines);
        assert!(text.contains("edit #1"), "text={text}");
        assert!(text.contains("- foo"), "text={text}");
        assert!(text.contains("+ bar"), "text={text}");
    }
}
