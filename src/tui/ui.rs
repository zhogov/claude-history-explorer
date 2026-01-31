use crate::tui::app::App;
use chrono::{DateTime, Local};
use chrono_humanize::{Accuracy, HumanTime, Tense};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

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

            // First line: [project] timestamp
            let mut header_spans = Vec::new();
            if let Some(ref name) = conv.project_name {
                header_spans.push(Span::styled(
                    format!("[{}] ", name),
                    Style::default().fg(Color::Cyan),
                ));
            }
            header_spans.push(Span::styled(
                timestamp,
                Style::default().fg(Color::DarkGray),
            ));
            let header = Line::from(header_spans);

            // Second line: preview text (sanitize newlines)
            let preview_style = if is_selected {
                Style::default()
            } else {
                Style::default().fg(Color::Gray)
            };
            let preview_text = conv.preview.replace('\n', " ");
            let preview = Line::from(Span::styled(preview_text, preview_style));

            // Combine into two-line item
            let content = vec![header, preview];

            let mut item = ListItem::new(content);
            if is_selected {
                item = item.style(Style::default().bg(Color::DarkGray));
            }

            item
        })
        .collect();

    // Calculate visible range to show selected item
    let items_per_page = (area.height as usize) / 2; // Two lines per item

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

    let list = List::new(visible_items).block(Block::default());

    frame.render_widget(list, area);

    // Render selection indicator
    if let Some(selected) = app.selected() {
        let visible_idx = selected - offset;
        let y = area.y + (visible_idx * 2) as u16;
        if y < area.y + area.height {
            let indicator = Paragraph::new("▶").style(Style::default().fg(Color::Yellow));
            let indicator_area = Rect::new(area.x, y, 1, 1);
            frame.render_widget(indicator, indicator_area);
        }
    }
}

fn format_relative_time(timestamp: DateTime<Local>) -> String {
    let delta = timestamp.signed_duration_since(Local::now());
    HumanTime::from(delta).to_text_en(Accuracy::Rough, Tense::Present)
}
