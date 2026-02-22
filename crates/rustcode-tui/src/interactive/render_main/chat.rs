use super::super::{
    apply_find_highlight, approval_options_count, build_transcript_lines, composer_cursor_visual,
    render_activity, render_activity_details_modal, render_approval_inline,
    render_approval_selector, word_wrap_text, AppState, ApprovalMode, Block, Borders, ChatFocus,
    ChatState, Clear, Constraint, Direction, Layout, Line, Modal, Modifier, Paragraph, Span, Style,
    Wrap,
};
use super::{format_tokens, truncate_with_ellipsis};
use crate::interactive::theme;

mod footer;
use self::footer::build_footer_lines;

pub(super) fn render_chat(frame: &mut ratatui::Frame<'_>, app: &AppState, chat: &ChatState) {
    // Auto-hide activity panel on narrow terminals (< 60 cols).
    let effective_hidden = chat.activity_hidden || frame.area().width < 60;
    let h_constraints: Vec<Constraint> = if effective_hidden {
        vec![Constraint::Percentage(100)]
    } else {
        vec![Constraint::Percentage(72), Constraint::Percentage(28)]
    };
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(h_constraints)
        .split(frame.area());

    // Dynamic composer height: expands with content including visual wrapping.
    // Pre-compute inner width from the left pane (frame width * percentage - 2 borders).
    let left_pane_w = if effective_hidden {
        frame.area().width
    } else {
        frame.area().width * 72 / 100
    };
    let composer_inner_w = left_pane_w.saturating_sub(2).max(1) as usize;
    let composer_visual_lines = if chat.composer.is_empty() {
        1
    } else {
        word_wrap_text(&chat.composer, composer_inner_w).len()
    };
    let composer_height = (composer_visual_lines as u16 + 2).max(5);

    // When the approval selector is shown, enlarge the bottom pane to fit
    // the vertical option list (options + hint line + 2 borders).
    let bottom_height = if let Some(pending) = &app.pending_approval {
        let opt_count = approval_options_count(&pending.request) as u16;
        opt_count + 3 // options + hint + top/bottom borders
    } else {
        composer_height
    };

    // Clamp the bottom pane so the transcript always gets at least 3 rows.
    let available = root[0].height;
    let fixed_h = 1u16 + 3; // mode bar + status bar
    let remaining = available.saturating_sub(fixed_h);
    let clamped_bottom = bottom_height.min(remaining.saturating_sub(3));

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(remaining.saturating_sub(clamped_bottom).max(1)), // [0] transcript
            Constraint::Length(1),                                            // [1] mode bar
            Constraint::Length(clamped_bottom), // [2] composer / approval
            Constraint::Length(3),              // [3] status bar
        ])
        .split(root[0]);

    // Extract provider name from config or model string (e.g., "openrouter" or "opencode/gpt-4o")
    let provider_label = {
        let mut provider_str = app.defaults.provider.trim();
        if provider_str.is_empty() {
            let model = app.defaults.model.as_str();
            if let Some((p, _)) = model.split_once('/') {
                provider_str = p;
            } else {
                provider_str = model;
            }
        }

        if provider_str.is_empty() || provider_str.eq_ignore_ascii_case("null") {
            None
        } else if provider_str.eq_ignore_ascii_case("openai") {
            Some("OpenAI".to_string())
        } else {
            // Title-case the provider name
            let mut chars = provider_str.chars();
            chars.next().map(|first| {
                let mut s = first.to_uppercase().to_string();
                s.extend(chars);
                s
            })
        }
    };
    // Build transcript title spans with distinct colors per metric.
    let mut transcript_title_spans = vec![
        // App brand
        Span::styled(
            "RustCode",
            Style::default()
                .fg(theme::SECONDARY)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if chat.last_total_tokens > 0 {
        let tokens = format_tokens(chat.last_total_tokens);
        transcript_title_spans.push(Span::raw("  "));
        transcript_title_spans.push(Span::styled(
            format!("{tokens} tokens"),
            Style::default().fg(theme::TOKENS),
        ));
    }
    if chat.context_limit > 0 && chat.last_total_tokens > 0 {
        // Exclude cache reads from context usage — cached tokens don't consume
        // new context window space, so the % should reflect "real" usage.
        let effective = chat
            .last_total_tokens
            .saturating_sub(chat.cache_read_tokens);
        let pct = ((effective as f64 / chat.context_limit as f64) * 100.0).min(100.0);
        let pct_color = if pct < 50.0 {
            theme::CTX_LOW
        } else if pct < 75.0 {
            theme::CTX_MED
        } else {
            theme::CTX_HIGH
        };
        transcript_title_spans.push(Span::raw("  "));
        transcript_title_spans.push(Span::styled(
            format!("{pct:.0}% ctx"),
            Style::default().fg(pct_color),
        ));
    }
    if chat.cache_read_tokens > 0 {
        let cached = format_tokens(chat.cache_read_tokens);
        transcript_title_spans.push(Span::raw("  "));
        transcript_title_spans.push(Span::styled(
            format!("{cached} cached"),
            Style::default().fg(theme::MUTED),
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

    let transcript_inner_h = left[0].height.saturating_sub(2).max(1) as usize;
    let inner_w = left[0].width.saturating_sub(2).max(1) as usize;

    // ── Transcript caching: only rebuild when dirty ────────────────────
    // Always rebuild while a run is active — the transcript contains animated
    // spinners and elapsed timers that update with `SystemTime::now()`.
    let needs_rebuild = chat.transcript_dirty.get() || chat.running.is_some();
    let width_changed = chat.last_transcript_width.get() != inner_w;

    let mut lines = if needs_rebuild {
        let built = build_transcript_lines(chat);
        *chat.cached_transcript.borrow_mut() = built.clone();
        chat.transcript_dirty.set(false);
        built
    } else {
        chat.cached_transcript.borrow().clone()
    };
    lines = apply_find_highlight(lines, chat.find.as_ref());

    // Append inline approval previews: committed (approved, awaiting result) first,
    // then the current pending approval (if any).
    // Committed approvals are cached and only re-rendered when width changes.
    {
        let inner_w16 = left[0].width.saturating_sub(2);
        let ws_root = app
            .config
            .as_ref()
            .map(|c| c.workspace_root.clone())
            .unwrap_or_default();
        for ca in &chat.committed_approvals {
            if ca.width == inner_w16 && !ca.lines.is_empty() {
                lines.extend(ca.lines.clone());
            } else {
                lines.extend(render_approval_inline(&ca.request, inner_w16, &ws_root));
            }
        }
        if let Some(pending) = &app.pending_approval {
            lines.extend(render_approval_inline(
                &pending.request,
                inner_w16,
                &ws_root,
            ));
        }
    }

    // Pad diff lines so the background colour fills the entire row width.
    // Only re-pad when width changes or transcript was rebuilt.
    if needs_rebuild || width_changed {
        for line in &mut lines {
            if let Some(bg_color) = line.style.bg {
                let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
                if inner_w > used {
                    line.spans.push(Span::styled(
                        " ".repeat(inner_w - used),
                        Style::default().bg(bg_color),
                    ));
                }
            }
        }
        chat.last_transcript_width.set(inner_w);
    }

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
    let max_scroll = wrapped_count.saturating_sub(transcript_inner_h);
    // Update Cells so compute_desired_height and the scroll handler can
    // read accurate values without re-running build_transcript_lines.
    chat.last_max_scroll.set(max_scroll);
    chat.last_transcript_wrapped_count.set(wrapped_count);
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
        Style::default().fg(theme::MUTED),
    ))
    .right_aligned();

    let transcript_block = Block::default()
        .title(Line::from(transcript_title_spans))
        .title_bottom(transcript_bottom)
        .borders(Borders::ALL);
    let transcript = Paragraph::new(lines)
        .block(transcript_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_top.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(Clear, left[0]);
    frame.render_widget(transcript, left[0]);

    // ─── Mode bar (1 row between transcript and composer) ────────────────
    {
        let mode = app.approval_mode;
        let mode_style = match mode {
            ApprovalMode::Yolo => Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
            ApprovalMode::AcceptEdits => Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
            ApprovalMode::Plan => Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
            ApprovalMode::Normal => Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        };
        let icon_label = format!("{} {}", mode.icon(), mode.label());
        let mut mode_spans = vec![Span::raw(" "), Span::styled(icon_label, mode_style)];
        // Hide the keyboard hint on narrow terminals (< 30 cols).
        if frame.area().width >= 30 {
            mode_spans.push(Span::raw(" "));
            mode_spans.push(Span::styled(
                "(Shift + Tab)",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::DIM),
            ));
        }
        let mode_line = Line::from(mode_spans);
        frame.render_widget(Paragraph::new(mode_line), left[1]);
    }

    let term_w = frame.area().width;
    let model_label = if chat.session.model.trim().is_empty() {
        app.defaults.model.as_str()
    } else {
        chat.session.model.as_str()
    };
    // Adaptive truncation: shorter model label on narrow terminals.
    let model_max = if term_w < 30 { 16 } else { 36 };
    let model_label_display = truncate_with_ellipsis(model_label, model_max);
    // Hide provider label entirely when < 25 cols.
    let provider_label_display = if term_w >= 25 {
        provider_label
            .as_deref()
            .map(|label| truncate_with_ellipsis(label, 18))
    } else {
        None
    };

    let mut composer_title_spans = vec![
        Span::styled("</>", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            format!("[model:{model_label_display}]"),
            Style::default()
                .fg(theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(provider_label_display) = provider_label_display {
        composer_title_spans.push(Span::raw("  "));
        composer_title_spans.push(Span::styled(
            format!("[{provider_label_display}]"),
            Style::default()
                .fg(theme::INFO)
                .add_modifier(Modifier::BOLD),
        ));
    }
    // Show character count when the composer has content
    if !chat.composer.is_empty() {
        composer_title_spans.push(Span::raw("  "));
        composer_title_spans.push(Span::styled(
            format!("[{}c]", chat.composer.chars().count()),
            Style::default()
                .fg(theme::MUTED)
                .add_modifier(Modifier::DIM),
        ));
    }
    let composer_title = Line::from(composer_title_spans);
    let composer_title_width = composer_title.width();
    let composer_border = if chat.focus == ChatFocus::Composer {
        Style::default().fg(theme::BORDER_FOCUS)
    } else {
        Style::default().fg(theme::BORDER)
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
        render_approval_selector(frame, left[2], &pending.request, app.approval_selection);
    } else {
        let composer_area = left[2];
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
                    Style::default().fg(theme::MUTED),
                ));
                if stat.insertions > 0 {
                    git_spans.push(Span::raw(" "));
                    git_spans.push(Span::styled(
                        format!("+{}", stat.insertions),
                        Style::default().fg(theme::SUCCESS),
                    ));
                }
                if stat.deletions > 0 {
                    git_spans.push(Span::raw(" "));
                    git_spans.push(Span::styled(
                        format!("-{}", stat.deletions),
                        Style::default().fg(theme::ERROR),
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
        // Manually character-wrap the composer text so the visual layout
        // matches `composer_cursor_visual` exactly.  Using ratatui's
        // `Wrap { trim: false }` would word-wrap, causing a mismatch
        // between the computed cursor position and the rendered text when
        // the composer is narrow (e.g. activity panel open).
        let wrapped_lines = word_wrap_text(composer_text, inner_w.max(1) as usize);
        let visible: Vec<Line<'_>> = wrapped_lines
            .into_iter()
            .skip(composer_scroll)
            .take(inner_h as usize)
            .map(|s| Line::from(Span::styled(s, composer_style)))
            .collect();
        let composer = Paragraph::new(visible).block(composer_block);
        frame.render_widget(composer, composer_area);

        // Render a block cursor when composing (reversed cell at cursor position).
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
            theme::render_block_cursor(frame, x, y, composer_area);
        }
    }

    let latest_toast = app
        .toasts
        .last()
        .map(|toast| (toast.message.clone(), toast.variant.clone()));
    let has_error = false;
    let composer_starts_with_query = chat.composer.starts_with('?');
    let (status_line, hints_line) = build_footer_lines(
        chat.running.is_some(),
        latest_toast,
        composer_starts_with_query,
        effective_hidden,
        has_error,
        frame.area().width,
    );

    let help = Paragraph::new(vec![status_line, hints_line]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::MUTED)),
    );
    frame.render_widget(help, left[3]);

    if !effective_hidden {
        // Constrain the activity panel to not extend below the status bar.
        // The status bar (3 rows) + mode bar (1 row) occupy the bottom of the left pane.
        let activity_area = {
            let full = root[1];
            let status_h = 3u16 + 1; // status bar + mode bar
            let h = full.height.saturating_sub(status_h);
            ratatui::layout::Rect::new(full.x, full.y, full.width, h)
        };
        render_activity(frame, activity_area, chat);
    }
    if chat.details_open {
        render_activity_details_modal(frame, chat);
    }
}

// `word_wrap_text` lives in composer.rs alongside `composer_cursor_visual`
// so the wrapping logic stays in one place.
