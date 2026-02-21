use super::super::{
    apply_find_highlight, approval_options_count, build_transcript_lines, composer_cursor_visual,
    render_activity, render_activity_details_modal, render_approval_inline,
    render_approval_selector, AppState, Block, Borders, ChatFocus, ChatState, Clear, Color,
    Constraint, Direction, Duration, InteractiveSubmitMode, Layout, Line, Modal, Modifier,
    Paragraph, Span, Style, SystemTime, ToastVariant, Wrap,
};
use super::{format_tokens, truncate_with_ellipsis};

const RUNNING_FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
const RUNNING_DOTS: &[&str] = &["   ", ".  ", ".. ", "..."];

pub(super) fn render_chat(frame: &mut ratatui::Frame<'_>, app: &AppState, chat: &ChatState) {
    let h_constraints: Vec<Constraint> = if chat.activity_hidden {
        vec![Constraint::Percentage(100)]
    } else {
        vec![Constraint::Percentage(72), Constraint::Percentage(28)]
    };
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(h_constraints)
        .split(frame.area());

    // Dynamic composer height: expands with newlines (min 5 = 3 content + 2 borders,
    // max 8 = 6 content + 2 borders).
    let composer_content_lines = if chat.composer.is_empty() {
        1
    } else {
        chat.composer.lines().count().max(1)
    };
    let composer_height = (composer_content_lines as u16 + 2).clamp(5, 8);

    // When the approval selector is shown, enlarge the bottom pane to fit
    // the vertical option list (options + hint line + 2 borders).
    let bottom_height = if let Some(pending) = &app.pending_approval {
        let opt_count = approval_options_count(&pending.request) as u16;
        opt_count + 3 // options + hint + top/bottom borders
    } else {
        composer_height
    };

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(bottom_height),
            Constraint::Length(3),
        ])
        .split(root[0]);

    // Extract provider name from config or model string (e.g., "openrouter" or "opencode/gpt-4o")
    let provider_label = {
        let mut provider_str = app.defaults.provider.as_str();
        if provider_str.is_empty() {
            let model = app.defaults.model.as_str();
            if let Some((p, _)) = model.split_once('/') {
                provider_str = p;
            } else {
                provider_str = model;
            }
        }

        // Title-case the provider name
        let mut chars = provider_str.chars();
        match chars.next() {
            Some(first) => {
                let mut s = first.to_uppercase().to_string();
                s.extend(chars);
                s
            }
            None => app.defaults.model.clone(),
        }
    };
    // Build transcript title spans with distinct colors per metric.
    let mut transcript_title_spans = vec![
        // App brand
        Span::styled(
            "RustCode",
            Style::default()
                .fg(Color::Rgb(180, 120, 240))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if chat.last_total_tokens > 0 {
        let tokens = format_tokens(chat.last_total_tokens);
        transcript_title_spans.push(Span::raw("  "));
        transcript_title_spans.push(Span::styled(
            format!("{tokens} tokens"),
            Style::default().fg(Color::Rgb(160, 210, 160)),
        ));
    }
    if chat.context_limit > 0 && chat.last_total_tokens > 0 {
        let pct = ((chat.last_total_tokens as f64 / chat.context_limit as f64) * 100.0).min(100.0);
        let pct_color = if pct < 50.0 {
            Color::Rgb(80, 200, 80)
        } else if pct < 75.0 {
            Color::Rgb(220, 200, 60)
        } else {
            Color::Rgb(220, 110, 60)
        };
        transcript_title_spans.push(Span::raw("  "));
        transcript_title_spans.push(Span::styled(
            format!("{pct:.0}% used"),
            Style::default().fg(pct_color),
        ));
    }
    if let Some(find) = &chat.find {
        if !find.query.trim().is_empty() {
            transcript_title_spans.push(Span::raw("  "));
            transcript_title_spans.push(Span::styled(
                format!(
                    "find: {} ({}/{})",
                    find.query,
                    find.current.saturating_add(1),
                    find.matches.len()
                ),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
    }

    let mut lines = build_transcript_lines(chat);
    lines = apply_find_highlight(lines, chat.find.as_ref());

    // Append inline approval info when a tool approval is pending.
    if let Some(pending) = &app.pending_approval {
        let inner_w = left[0].width.saturating_sub(2);
        let ws_root = app
            .config
            .as_ref()
            .map(|c| c.workspace_root.clone())
            .unwrap_or_default();
        lines.extend(render_approval_inline(&pending.request, inner_w, &ws_root));
    }

    let transcript_inner_h = left[0].height.saturating_sub(2).max(1) as usize;
    let inner_w = left[0].width.saturating_sub(2).max(1) as usize;
    // Account for line wrapping: each logical line may span multiple visual rows.
    let wrapped_count: usize = lines
        .iter()
        .map(|l| {
            let w = l.width();
            if w <= inner_w {
                1
            } else {
                w.div_ceil(inner_w)
            }
        })
        .sum();
    let max_scroll = wrapped_count
        .saturating_sub(transcript_inner_h)
        .min(u16::MAX as usize) as u16;
    // Update Cell so the scroll event handler can clamp immediately.
    chat.last_max_scroll.set(max_scroll);
    let from_bottom = chat.scroll.min(max_scroll);
    let scroll_top = max_scroll.saturating_sub(from_bottom);

    // Build workspace + version footer for transcript bottom-right.
    let version = env!("CARGO_PKG_VERSION");
    let workspace_path = &app.defaults.workspace_root;
    let workspace_display = if let Ok(home) = std::env::var("HOME") {
        if workspace_path.starts_with(&home) {
            let suffix = workspace_path.strip_prefix(&home).unwrap_or(workspace_path);
            format!("~/{}", suffix.display())
        } else {
            workspace_path.display().to_string()
        }
    } else {
        workspace_path.display().to_string()
    };
    let workspace_display = workspace_display.replace("//", "/");
    let transcript_bottom = Line::from(Span::styled(
        format!("{workspace_display}  v{version}"),
        Style::default().fg(Color::DarkGray),
    ))
    .right_aligned();

    let transcript_block = Block::default()
        .title(Line::from(transcript_title_spans))
        .title_bottom(transcript_bottom)
        .borders(Borders::ALL);
    let transcript = Paragraph::new(lines)
        .block(transcript_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_top, 0));
    frame.render_widget(Clear, left[0]);
    frame.render_widget(transcript, left[0]);

    let mode_label = match app.submit_mode {
        InteractiveSubmitMode::Agent => "agent",
        InteractiveSubmitMode::Run => "run",
    };
    let model_label = if chat.session.model.trim().is_empty() {
        app.defaults.model.as_str()
    } else {
        chat.session.model.as_str()
    };
    let model_label_display = truncate_with_ellipsis(model_label, 36);
    let provider_label_display = truncate_with_ellipsis(&provider_label, 18);

    let prompt_label = if chat.running.is_some() {
        "</> Prompt (running)"
    } else {
        "</> Prompt"
    };
    let mut composer_title_spans = vec![
        Span::styled(prompt_label, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            format!("[model:{model_label_display}]"),
            Style::default()
                .fg(Color::Rgb(60, 210, 120))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[mode:{mode_label}]"),
            Style::default()
                .fg(Color::Rgb(240, 190, 55))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{provider_label_display}]"),
            Style::default()
                .fg(Color::Rgb(100, 160, 220))
                .add_modifier(Modifier::BOLD),
        ),
    ];
    // Show character count when the composer has content
    if !chat.composer.is_empty() {
        composer_title_spans.push(Span::raw("  "));
        composer_title_spans.push(Span::styled(
            format!("[{}c]", chat.composer.chars().count()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }
    let composer_title = Line::from(composer_title_spans);
    let composer_title_width = composer_title.width();
    let composer_border = if chat.focus == ChatFocus::Composer {
        Style::default().fg(Color::Rgb(80, 220, 220))
    } else {
        Style::default().fg(Color::Rgb(60, 70, 85))
    };
    let composer_text = if chat.composer.is_empty() {
        "Type a prompt... (Enter to submit, Shift+Enter for newline)"
    } else {
        chat.composer.as_str()
    };
    let composer_style = if chat.composer.is_empty() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default()
    };
    if let Some(pending) = &app.pending_approval {
        render_approval_selector(frame, left[1], &pending.request, app.approval_selection);
    } else {
        let composer_area = left[1];
        let inner_w = composer_area.width.saturating_sub(2);
        let inner_h = composer_area.height.saturating_sub(2);
        let (cursor_row, cursor_col) = if chat.focus == ChatFocus::Composer {
            composer_cursor_visual(&chat.composer, chat.composer_cursor, inner_w)
        } else {
            (0, 0)
        };
        let composer_scroll = if chat.focus == ChatFocus::Composer {
            cursor_row.saturating_sub(inner_h.saturating_sub(1) as usize)
        } else {
            0
        };

        // Build the composer block with an optional right-aligned git stat title.
        let mut composer_block = Block::default()
            .title(composer_title)
            .borders(Borders::ALL)
            .border_style(composer_border);
        if let Some(stat) = app.git_stat.as_ref() {
            if stat.files > 0 {
                let mut git_spans = Vec::new();
                git_spans.push(Span::styled(
                    format!("{} files", stat.files),
                    Style::default().fg(Color::DarkGray),
                ));
                if stat.insertions > 0 {
                    git_spans.push(Span::raw(" "));
                    git_spans.push(Span::styled(
                        format!("+{}", stat.insertions),
                        Style::default().fg(Color::Rgb(80, 210, 80)),
                    ));
                }
                if stat.deletions > 0 {
                    git_spans.push(Span::raw(" "));
                    git_spans.push(Span::styled(
                        format!("-{}", stat.deletions),
                        Style::default().fg(Color::Rgb(210, 80, 80)),
                    ));
                }
                git_spans.push(Span::raw(" ")); // trailing padding inside border
                let git_title = Line::from(git_spans).right_aligned();
                let fits = composer_title_width + git_title.width() < inner_w as usize;
                if fits {
                    composer_block = composer_block.title(git_title);
                }
            }
        }

        let composer_inner = composer_block.inner(composer_area);
        let composer = Paragraph::new(composer_text)
            .style(composer_style)
            .block(composer_block)
            .wrap(Wrap { trim: false })
            .scroll((composer_scroll as u16, 0));
        frame.render_widget(composer, composer_area);

        // Show cursor when composing, even if the slash-help popup is open.
        let slash_help_open = matches!(&app.modal, Some(Modal::SlashHelp { .. }));
        if chat.focus == ChatFocus::Composer
            && !app.help_open
            && (app.modal.is_none() || slash_help_open)
            && !chat.details_open
        {
            let x = composer_inner.x.saturating_add(cursor_col as u16);
            let y = composer_inner
                .y
                .saturating_add((cursor_row.saturating_sub(composer_scroll)) as u16);
            if x < composer_area.x + composer_area.width
                && y < composer_area.y + composer_area.height
            {
                frame.set_cursor_position((x, y));
            }
        }
    }

    // Line 1: running/toast status only (errors are shown in transcript).
    let (status_text, status_style, has_error) = if let Some(toast) = app.toasts.last() {
        let style = match toast.variant {
            ToastVariant::Info => Style::default().fg(Color::Cyan),
            ToastVariant::Success => Style::default().fg(Color::Green),
            ToastVariant::Warning => Style::default().fg(Color::Magenta),
            ToastVariant::Error => Style::default().fg(Color::Red),
        };
        (toast.message.clone(), style, false)
    } else {
        (String::new(), Style::default(), false)
    };
    // Bright yellow hint when error is set — draws attention
    let error_hint = if has_error {
        Span::styled(
            "  Ctrl+E:details",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    let mut status_spans = Vec::new();
    if chat.running.is_some() {
        let ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis();
        let frame_char = RUNNING_FRAMES[(ms / 120 % RUNNING_FRAMES.len() as u128) as usize];
        let dots = RUNNING_DOTS[(ms / 300 % RUNNING_DOTS.len() as u128) as usize];
        status_spans.push(Span::styled(
            format!(" {frame_char} running{dots}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    status_spans.push(Span::styled(status_text, status_style));
    status_spans.push(error_hint);
    let status_line = Line::from(status_spans);

    // When the composer starts with '?' show expanded bindings; otherwise a compact hint.
    let composer_starts_with_query = chat.composer.starts_with('?');
    let hints_line = if composer_starts_with_query {
        // Expanded bindings visible when user types '?' first
        let mut spans = vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                "Ctrl+P",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cmds  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+N",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":new  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+Q",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":sessions  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "Ctrl+C",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":cancel  ", Style::default().fg(Color::DarkGray)),
        ];
        if !chat.activity_hidden {
            spans.push(Span::styled(
                "Alt+Tab",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                ":switch focus  ",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                "Ctrl+W",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                ":toggle activity  ",
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            "/",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(if has_error {
            Span::styled(":cmd  Ctrl+E:error", Style::default().fg(Color::DarkGray))
        } else {
            Span::styled(":cmd", Style::default().fg(Color::DarkGray))
        });
        spans.push(Span::styled(
            "  Ctrl+O:toggle tools",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
        Line::from(spans)
    } else {
        // Compact hint — just enough to orient a new user
        let error_part = if has_error {
            Span::styled(
                "  Ctrl+E:error",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        };
        let mut spans = vec![
            Span::styled(" Ctrl+P", Style::default().fg(Color::DarkGray)),
            Span::styled(" cmds", Style::default().fg(Color::DarkGray)),
            Span::styled("   /", Style::default().fg(Color::DarkGray)),
            Span::styled(" cmd", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "   ? bindings",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ];
        if !chat.activity_hidden {
            spans.push(Span::styled(
                "   Alt+Tab",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                " switch focus",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                "   Ctrl+W",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                " toggle activity",
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(error_part);
        Line::from(spans)
    };

    let help = Paragraph::new(vec![status_line, hints_line]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(help, left[2]);

    if !chat.activity_hidden {
        render_activity(frame, root[1], chat);
    }
    if chat.details_open {
        render_activity_details_modal(frame, chat);
    }
}
