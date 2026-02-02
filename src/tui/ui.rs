use crate::tui::app::{App, LoadingState, Mode};
use crate::tui::search::is_word_separator;
use chrono::{DateTime, Local};
use chrono_humanize::{Accuracy, HumanTime, Tense};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph};

/// Lines per conversation item (header + preview + separator)
const LINES_PER_ITEM: usize = 3;

/// Render the TUI
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Outer border wrapping the entire app
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(60, 60, 60)));
    let inner_area = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    // Check if we need space for confirmation dialog
    let (list_area, confirm_area) = if *app.mode() == Mode::ConfirmDelete {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner_area);
        render_search_bar(frame, app, chunks[0]);
        (chunks[1], Some(chunks[2]))
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner_area);
        render_search_bar(frame, app, chunks[0]);
        (chunks[1], None)
    };

    render_list(frame, app, list_area);

    if let Some(area) = confirm_area {
        render_confirm_dialog(frame, area);
    }
}

fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let status_text = match app.loading_state() {
        LoadingState::Loading { loaded } => {
            format!("Loading... {}", loaded)
        }
        LoadingState::Ready => {
            format!("{}/{}", app.filtered().len(), app.conversations().len())
        }
    };

    // Build search line: " ❯ query" on left, "status " on right
    let prompt = " ❯ ";
    let query = app.query();
    let left_len = prompt.chars().count() + query.chars().count();
    let count_len = status_text.chars().count() + 1; // +1 for trailing space
    let padding = (area.width as usize).saturating_sub(left_len + count_len + 1);

    // Prompt is always active - user can type during loading
    let prompt_style = Style::default().fg(Color::Rgb(78, 201, 176));

    let status_style = if app.is_loading() {
        Style::default().fg(Color::Rgb(78, 201, 176)) // Highlight loading status
    } else {
        Style::default().fg(Color::Rgb(100, 100, 100))
    };

    let search_line = Line::from(vec![
        Span::raw(" "),
        Span::styled("❯ ", prompt_style),
        Span::raw(query.to_string()),
        Span::raw(" ".repeat(padding)),
        Span::styled(status_text, status_style),
        Span::raw(" "),
    ]);

    let input = Paragraph::new(search_line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Rgb(60, 60, 60))),
    );

    frame.render_widget(input, area);

    // Position cursor at cursor_pos (account for " ❯ " prefix)
    if area.width > 3 {
        let cursor_offset = app.cursor_pos() as u16;
        let max_x = area.x + area.width.saturating_sub(2);
        let cursor_x = (area.x + 3).saturating_add(cursor_offset).min(max_x);
        frame.set_cursor_position(Position::new(cursor_x, area.y));
    }
}

fn render_confirm_dialog(frame: &mut Frame, area: Rect) {
    let prompt = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "Delete this conversation? ",
            Style::default().fg(Color::Yellow),
        ),
        Span::styled("(y/n)", Style::default().fg(Color::Rgb(140, 140, 140))),
    ]);
    let paragraph = Paragraph::new(prompt);
    frame.render_widget(paragraph, area);
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    // Use cached query words instead of reparsing
    let query_words: Vec<&str> = app.query_words().iter().map(|s| s.as_str()).collect();

    // Calculate visible range FIRST (before building any items)
    // When searching, items may have 4 lines (with context), so use 4 lines per item
    // to ensure the offset calculation matches the actual rendered heights
    let lines_per_item = if query_words.is_empty() {
        LINES_PER_ITEM // 3 lines: header, preview, separator
    } else {
        4 // 4 lines: header, preview, context (optional but reserve space), separator
    };
    let items_per_page = (area.height as usize) / lines_per_item;
    let offset = match (app.selected(), items_per_page) {
        (Some(sel), n) if n > 0 => (sel / n) * n,
        _ => 0,
    };
    let visible_count = items_per_page.max(1);

    // Cache separator string (same for all items in this frame)
    let separator_str = "─".repeat(width);

    // Only build ListItems for the visible range
    let visible_items: Vec<ListItem> = app
        .filtered()
        .iter()
        .skip(offset)
        .take(visible_count)
        .enumerate()
        .map(|(relative_idx, &conv_idx)| {
            let list_idx = offset + relative_idx;
            let conv = &app.conversations()[conv_idx];
            let is_selected = app.selected() == Some(list_idx);

            // Format timestamp
            let timestamp = if app.use_relative_time() {
                format_relative_time(conv.timestamp)
            } else {
                conv.timestamp.format("%b %d, %H:%M").to_string()
            };

            // Format message count
            let msg_count = if conv.message_count == 1 {
                "1 msg".to_string()
            } else {
                format!("{} msgs", conv.message_count)
            };

            // Selection indicator: vertical bar for all rows (with left padding)
            let indicator = " ▌ ";
            let indicator_style = if is_selected {
                Style::default().fg(Color::Rgb(78, 201, 176))
            } else {
                Style::default().fg(Color::Rgb(60, 60, 60))
            };

            // Build left part: indicator + project
            let project_part = conv
                .project_name
                .as_ref()
                .map(|name| name.to_string())
                .unwrap_or_default();

            // Calculate padding for right-aligned timestamp + message count
            let left_len = indicator.chars().count() + project_part.chars().count();
            let right_len = msg_count.chars().count() + 3 + timestamp.chars().count(); // 3 for " · "
            let padding = width.saturating_sub(left_len + right_len + 1);

            // Header line: ▌ project-name                    timestamp
            let project_style = if is_selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::White)
            };

            // Highlight style: cyan with bold for selected row
            let highlight_style = if is_selected {
                Style::default().fg(Color::Rgb(78, 201, 176)).bold()
            } else {
                Style::default().fg(Color::Rgb(78, 201, 176))
            };

            let selection_bg = if is_selected {
                Style::default().bg(Color::Rgb(45, 45, 55))
            } else {
                Style::default()
            };

            // Build header with highlighted project name
            let mut header_spans = vec![Span::styled(indicator, indicator_style)];
            header_spans.extend(highlight_text(
                &project_part,
                &query_words,
                project_style,
                highlight_style,
            ));
            header_spans.push(Span::raw(" ".repeat(padding)));
            header_spans.push(Span::styled(
                msg_count,
                Style::default().fg(Color::Rgb(110, 110, 110)),
            ));
            header_spans.push(Span::styled(
                " · ",
                Style::default().fg(Color::Rgb(70, 70, 70)),
            ));
            header_spans.push(Span::styled(
                timestamp,
                Style::default().fg(Color::Rgb(140, 140, 140)),
            ));

            let header = Line::from(header_spans).style(selection_bg);

            // Preview line: sanitized and truncated
            let preview_text = sanitize_preview(&conv.preview);
            let max_preview_len = width.saturating_sub(4);
            let truncated_preview = if preview_text.chars().count() > max_preview_len {
                let truncated: String = preview_text
                    .chars()
                    .take(max_preview_len.saturating_sub(1))
                    .collect();
                format!("{}…", truncated)
            } else {
                preview_text
            };

            // Build preview with highlighted matches
            let preview_style = Style::default().fg(Color::Rgb(130, 130, 130));
            let mut preview_spans = vec![Span::styled(indicator, indicator_style)];
            preview_spans.extend(highlight_text(
                &truncated_preview,
                &query_words,
                preview_style,
                highlight_style,
            ));

            let preview = Line::from(preview_spans).style(selection_bg);

            // Check for hidden matches and build context line if needed
            let context_line = if !query_words.is_empty() {
                if let Some((match_pos, match_char_len)) =
                    find_hidden_match(&conv.full_text, &truncated_preview, &query_words)
                {
                    let context_width = width.saturating_sub(4); // Account for indicator
                    let context_text = extract_match_context(
                        &conv.full_text,
                        match_pos,
                        match_char_len,
                        context_width,
                    );

                    // Truncate context if still too long
                    let truncated_context = if context_text.chars().count() > context_width {
                        let truncated: String = context_text
                            .chars()
                            .take(context_width.saturating_sub(1))
                            .collect();
                        format!("{}…", truncated)
                    } else {
                        context_text
                    };

                    // Build context line with highlighting (dimmer style)
                    let context_base_style = Style::default().fg(Color::Rgb(100, 100, 100));
                    let context_highlight_style = Style::default().fg(Color::Rgb(60, 160, 140)); // Dimmer cyan

                    let mut context_spans = vec![Span::styled(indicator, indicator_style)];
                    context_spans.extend(highlight_text(
                        &truncated_context,
                        &query_words,
                        context_base_style,
                        context_highlight_style,
                    ));

                    Some(Line::from(context_spans).style(selection_bg))
                } else {
                    None
                }
            } else {
                None
            };

            // Separator line: dim horizontal rule (full width)
            let separator = Line::from(Span::styled(
                separator_str.as_str(),
                Style::default().fg(Color::Rgb(50, 50, 50)),
            ));

            // Combine into item (3 or 4 lines depending on context)
            let lines = if let Some(ctx) = context_line {
                vec![header, preview, ctx, separator]
            } else {
                vec![header, preview, separator]
            };

            ListItem::new(lines)
        })
        .collect();

    let list = List::new(visible_items);
    frame.render_widget(list, area);
}

fn format_relative_time(timestamp: DateTime<Local>) -> String {
    let delta = timestamp.signed_duration_since(Local::now());
    HumanTime::from(delta).to_text_en(Accuracy::Rough, Tense::Present)
}

/// Sanitize preview text by removing XML-like tags and normalizing whitespace
fn sanitize_preview(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            '\n' | '\r' | '\t' => {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            ' ' => {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            _ => {
                result.push(ch);
                last_was_space = false;
            }
        }
    }

    result.trim().to_string()
}

/// Split text into spans with matched portions highlighted (case-insensitive)
fn highlight_text(
    text: &str,
    query_words: &[&str],
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    if query_words.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Collect chars for iteration (original text only)
    let chars: Vec<char> = text.chars().collect();

    // Build char-to-byte mapping
    let char_to_byte: Vec<usize> = text
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .chain(std::iter::once(text.len()))
        .collect();

    let mut spans = Vec::new();
    let mut last_end = 0;
    let mut char_idx = 0;

    while char_idx < chars.len() {
        // Check if we're at a word start
        let at_word_start = if char_idx == 0 {
            !is_word_separator(chars[char_idx])
        } else {
            is_word_separator(chars[char_idx - 1]) && !is_word_separator(chars[char_idx])
        };

        if at_word_start {
            // Find word end
            let word_start = char_idx;
            let mut word_end = char_idx;
            while word_end < chars.len() && !is_word_separator(chars[word_end]) {
                word_end += 1;
            }

            // Extract original word and lowercase it for comparison
            // This avoids Unicode index mismatch issues from to_lowercase changing char count
            let start_byte = char_to_byte[word_start];
            let end_byte = char_to_byte[word_end];
            let original_word = &text[start_byte..end_byte];
            let word_lower = original_word.to_lowercase();

            // Check if any query word is a prefix of this word
            let matched_query = query_words.iter().find(|&&qw| word_lower.starts_with(qw));

            if let Some(qw) = matched_query {
                // Add non-highlighted text before this word
                if word_start > last_end {
                    let prev_start_byte = char_to_byte[last_end];
                    spans.push(Span::styled(
                        text[prev_start_byte..start_byte].to_string(),
                        base_style,
                    ));
                }

                // Highlight the matched prefix portion
                // Use char count from original word to handle Unicode correctly
                let prefix_len = qw.chars().count();
                let highlight_end = (word_start + prefix_len).min(word_end);
                let highlight_end_byte = char_to_byte[highlight_end];
                spans.push(Span::styled(
                    text[start_byte..highlight_end_byte].to_string(),
                    highlight_style,
                ));

                // Add the rest of the word (unhighlighted)
                if highlight_end < word_end {
                    spans.push(Span::styled(
                        text[highlight_end_byte..end_byte].to_string(),
                        base_style,
                    ));
                }

                last_end = word_end;
            }
            char_idx = word_end;
        } else {
            char_idx += 1;
        }
    }

    // Add remaining text
    if last_end < chars.len() {
        let start_byte = char_to_byte[last_end];
        spans.push(Span::styled(text[start_byte..].to_string(), base_style));
    }

    if spans.is_empty() {
        vec![Span::styled(text.to_string(), base_style)]
    } else {
        spans
    }
}

/// Find the first match in full_text that is NOT visible in the preview.
/// Returns (byte_offset, matched_word_char_len) or None if all matches are visible.
fn find_hidden_match(
    full_text: &str,
    preview: &str,
    query_words: &[&str],
) -> Option<(usize, usize)> {
    if query_words.is_empty() {
        return None;
    }

    // Count word prefix matches in text using single-pass iteration
    // Uses original text chars and lowercases words individually to avoid Unicode index issues
    let count_word_matches = |text: &str| -> usize {
        let chars: Vec<char> = text.chars().collect();
        let char_to_byte: Vec<usize> = text
            .char_indices()
            .map(|(byte_idx, _)| byte_idx)
            .chain(std::iter::once(text.len()))
            .collect();

        let mut count = 0;
        let mut prev_sep = true;

        for (i, &c) in chars.iter().enumerate() {
            let is_sep = is_word_separator(c);
            if !is_sep && prev_sep {
                // Word start - find word end
                let word_end = chars[i..]
                    .iter()
                    .position(|&c| is_word_separator(c))
                    .map(|p| i + p)
                    .unwrap_or(chars.len());

                // Extract and lowercase word
                let start_byte = char_to_byte[i];
                let end_byte = char_to_byte[word_end];
                let word = &text[start_byte..end_byte];
                let word_lower = word.to_lowercase();

                if query_words.iter().any(|&qw| word_lower.starts_with(qw)) {
                    count += 1;
                }
            }
            prev_sep = is_sep;
        }
        count
    };

    let preview_matches = count_word_matches(preview);
    let full_matches = count_word_matches(full_text);

    if full_matches <= preview_matches {
        return None;
    }

    // Find the (preview_matches + 1)th match in full_text using single-pass
    // Use original text chars to avoid Unicode index mismatch
    let chars: Vec<char> = full_text.chars().collect();

    // Build char-to-byte mapping from original text
    let char_to_byte: Vec<usize> = full_text
        .char_indices()
        .map(|(byte_idx, _)| byte_idx)
        .chain(std::iter::once(full_text.len()))
        .collect();

    let mut match_count = 0;
    let mut prev_sep = true;

    for (i, &c) in chars.iter().enumerate() {
        let is_sep = is_word_separator(c);
        if !is_sep && prev_sep {
            // Word start
            let word_end = chars[i..]
                .iter()
                .position(|&c| is_word_separator(c))
                .map(|p| i + p)
                .unwrap_or(chars.len());

            // Extract and lowercase word
            let start_byte = char_to_byte[i];
            let end_byte = char_to_byte[word_end];
            let word = &full_text[start_byte..end_byte];
            let word_lower = word.to_lowercase();

            if let Some(&qw) = query_words.iter().find(|&&qw| word_lower.starts_with(qw)) {
                match_count += 1;
                if match_count > preview_matches {
                    // Return byte offset and the matched prefix length
                    return Some((start_byte, qw.chars().count()));
                }
            }
        }
        prev_sep = is_sep;
    }

    None
}

/// Extract a context snippet around a match position in full_text.
/// Returns a sanitized string with ellipsis prefix, suitable for display.
fn extract_match_context(
    full_text: &str,
    match_pos: usize,
    query_char_len: usize,
    max_width: usize,
) -> String {
    // Calculate how much context to show around the match
    // Reserve space for ellipsis prefix/suffix (2 chars each)
    let context_chars = max_width.saturating_sub(query_char_len).saturating_sub(4) / 2;

    // Find char boundaries around match_pos efficiently without collecting all chars
    // Go back ~context_chars characters from match_pos
    let start_byte = full_text[..match_pos.min(full_text.len())]
        .char_indices()
        .rev()
        .nth(context_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Go forward ~(query_char_len + context_chars) characters from match_pos
    let end_byte = full_text[match_pos.min(full_text.len())..]
        .char_indices()
        .nth(query_char_len + context_chars)
        .map(|(i, _)| match_pos + i)
        .unwrap_or(full_text.len());

    let snippet = &full_text[start_byte..end_byte.min(full_text.len())];

    // Sanitize the snippet (remove XML tags, normalize whitespace)
    let sanitized = sanitize_preview(snippet);

    // Add ellipsis prefix/suffix to indicate this is from elsewhere
    let prefix = if start_byte > 0 { "…" } else { "" };
    let suffix = if end_byte < full_text.len() {
        "…"
    } else {
        ""
    };

    format!("{}{}{}", prefix, sanitized, suffix)
}
