use crate::tui::app::App;
use chrono::{DateTime, Local};
use chrono_humanize::{Accuracy, HumanTime, Tense};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

/// Lines per conversation item (header + preview + separator)
const LINES_PER_ITEM: usize = 3;

/// Render the TUI
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Layout: search bar at top, list below
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    render_search_bar(frame, app, chunks[0]);
    render_list(frame, app, chunks[1]);
}

fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let result_count = format!("{}/{}", app.filtered().len(), app.conversations().len());
    let title = format!(" {} ", result_count);

    let input = Paragraph::new(format!("> {}", app.query()))
        .block(Block::default().borders(Borders::ALL).title(title));

    frame.render_widget(input, area);

    // Position cursor after the query text (clamped to area bounds)
    if area.width > 3 && area.height > 1 {
        let query_width = app.query().chars().count() as u16;
        let max_x = area.x + area.width.saturating_sub(2);
        let cursor_x = (area.x + 3).saturating_add(query_width).min(max_x);
        frame.set_cursor_position(Position::new(cursor_x, area.y + 1));
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let query = app.query().trim();

    let items: Vec<ListItem> = app
        .filtered()
        .iter()
        .enumerate()
        .map(|(list_idx, &conv_idx)| {
            let conv = &app.conversations()[conv_idx];
            let is_selected = app.selected() == Some(list_idx);

            // Format timestamp
            let timestamp = if app.use_relative_time() {
                format_relative_time(conv.timestamp)
            } else {
                conv.timestamp.format("%b %d, %H:%M").to_string()
            };

            // Selection indicator: vertical bar for all rows
            let indicator = "▌ ";
            let indicator_style = if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Rgb(60, 60, 60))
            };

            // Build left part: indicator + project
            let project_part = conv
                .project_name
                .as_ref()
                .map(|name| name.to_string())
                .unwrap_or_default();

            // Calculate padding for right-aligned timestamp
            let left_len = indicator.chars().count() + project_part.chars().count();
            let right_len = timestamp.chars().count();
            let padding = width.saturating_sub(left_len + right_len + 1);

            // Header line: ▌ project-name                    timestamp
            let project_style = if is_selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::White)
            };

            // Highlight style: bold for selected row, cyan for others
            let highlight_style = if is_selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::Rgb(78, 201, 176))
            };

            let selection_bg = if is_selected {
                Style::default().bg(Color::Rgb(45, 45, 55))
            } else {
                Style::default()
            };

            // Build header with highlighted project name
            let mut header_spans = vec![Span::styled(indicator.to_string(), indicator_style)];
            header_spans.extend(highlight_text(
                &project_part,
                query,
                project_style,
                highlight_style,
            ));
            header_spans.push(Span::raw(" ".repeat(padding)));
            header_spans.push(Span::styled(
                timestamp,
                Style::default().fg(Color::DarkGray),
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
            let preview_style = Style::default().fg(Color::Rgb(110, 110, 110));
            let mut preview_spans = vec![Span::styled(indicator.to_string(), indicator_style)];
            preview_spans.extend(highlight_text(
                &truncated_preview,
                query,
                preview_style,
                highlight_style,
            ));

            let preview = Line::from(preview_spans).style(selection_bg);

            // Separator line: dim horizontal rule
            let separator_char = "─".repeat(width.saturating_sub(2));
            let separator = Line::from(vec![
                Span::raw(" "),
                Span::styled(separator_char, Style::default().fg(Color::Rgb(50, 50, 50))),
            ]);

            // Combine into three-line item
            ListItem::new(vec![header, preview, separator])
        })
        .collect();

    // Calculate visible range to show selected item
    let items_per_page = (area.height as usize) / LINES_PER_ITEM;

    let offset = match (app.selected(), items_per_page) {
        (Some(sel), n) if n > 0 => (sel / n) * n,
        _ => 0,
    };

    // Create a list with the visible items
    let visible_items: Vec<ListItem> = items
        .into_iter()
        .skip(offset)
        .take(items_per_page.max(1))
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
    query: &str,
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut last_end = 0;

    for (start, _) in text_lower.match_indices(&query_lower) {
        // Add non-matching text before this match
        if start > last_end {
            spans.push(Span::styled(text[last_end..start].to_string(), base_style));
        }
        // Add the matched text with highlight
        let end = start + query.len();
        spans.push(Span::styled(text[start..end].to_string(), highlight_style));
        last_end = end;
    }

    // Add remaining non-matching text
    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_string(), base_style));
    }

    if spans.is_empty() {
        vec![Span::styled(text.to_string(), base_style)]
    } else {
        spans
    }
}
