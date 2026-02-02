//! Conversation viewer rendering for TUI display.
//!
//! This module renders conversation JSONL files to `Vec<RenderedLine>` for display
//! in the TUI viewer. It produces styled spans that ratatui can render directly,
//! without using ANSI escape codes.

use crate::claude::{AssistantMessage, ContentBlock, LogEntry, UserContent};
use crate::tui::app::{LineStyle, RenderedLine};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

const NAME_WIDTH: usize = 9;
const WHITE: (u8, u8, u8) = (255, 255, 255);
const TEAL: (u8, u8, u8) = (78, 201, 176);
const DIM_TEAL: (u8, u8, u8) = (60, 160, 140);
const SEPARATOR_COLOR: (u8, u8, u8) = (80, 80, 80);
const CYAN: (u8, u8, u8) = (0, 255, 255);
const CODE_COLOR: (u8, u8, u8) = (147, 161, 199);
const GREEN: (u8, u8, u8) = (0, 255, 0);
const BLUE: (u8, u8, u8) = (100, 149, 237);
const THINKING_TEXT: (u8, u8, u8) = (140, 145, 150);

/// Options for rendering a conversation
pub struct RenderOptions {
    pub show_tools: bool,
    pub show_thinking: bool,
    pub content_width: usize,
}

/// Render a conversation file to lines for display in the TUI viewer
pub fn render_conversation(
    file_path: &Path,
    options: &RenderOptions,
) -> std::io::Result<Vec<RenderedLine>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            render_entry(&mut lines, &entry, options);
        }
    }

    Ok(lines)
}

fn render_entry(lines: &mut Vec<RenderedLine>, entry: &LogEntry, options: &RenderOptions) {
    match entry {
        LogEntry::Summary { .. }
        | LogEntry::FileHistorySnapshot { .. }
        | LogEntry::System { .. }
        | LogEntry::Progress { .. } => {}
        LogEntry::User { message, .. } => {
            render_user_message(lines, message, options);
        }
        LogEntry::Assistant { message, .. } => {
            render_assistant_message(lines, message, options);
        }
    }
}

fn render_user_message(
    lines: &mut Vec<RenderedLine>,
    message: &crate::claude::UserMessage,
    options: &RenderOptions,
) {
    // Extract text from user message, collecting all text blocks
    let text = match &message.content {
        UserContent::String(s) => process_command_message(s),
        UserContent::Blocks(blocks) => {
            let texts: Vec<String> = blocks
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::Text { text } = block {
                        process_command_message(text)
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        }
    };

    if let Some(text) = text {
        let md_lines = render_markdown_to_lines(&text, options.content_width);
        render_ledger_block_styled(lines, "You", WHITE, true, md_lines);
        lines.push(RenderedLine { spans: vec![] }); // Empty line after message
    }
}

fn render_assistant_message(
    lines: &mut Vec<RenderedLine>,
    message: &AssistantMessage,
    options: &RenderOptions,
) {
    let mut printed = false;

    // Text blocks
    for block in &message.content {
        if let ContentBlock::Text { text } = block {
            let md_lines = render_markdown_to_lines(text, options.content_width);
            render_ledger_block_styled(lines, "Claude", TEAL, true, md_lines);
            printed = true;
        }
    }

    // Tool calls (if enabled)
    if options.show_tools {
        for block in &message.content {
            if let ContentBlock::ToolUse { name, input, .. } = block {
                let header = format!("<Calling: {}>", name);
                render_ledger_block_plain(lines, "Claude", DIM_TEAL, false, &header);
                if let Ok(formatted) = serde_json::to_string_pretty(input) {
                    render_continuation(lines, &formatted);
                }
                printed = true;
            }
        }
    }

    // Thinking blocks (if enabled)
    if options.show_thinking {
        for block in &message.content {
            if let ContentBlock::Thinking { thinking, .. } = block {
                let md_lines = render_markdown_to_lines(thinking, options.content_width);
                let styled_lines = apply_thinking_style(md_lines);
                render_ledger_block_styled(lines, "Thinking", DIM_TEAL, false, styled_lines);
                printed = true;
            }
        }
    }

    if printed {
        lines.push(RenderedLine { spans: vec![] });
    }
}

/// A line with styled spans from markdown rendering
struct StyledLine {
    spans: Vec<(String, LineStyle)>,
}

/// Render markdown text to styled lines for TUI display
fn render_markdown_to_lines(input: &str, max_width: usize) -> Vec<StyledLine> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(input, options);
    let mut renderer = TuiMarkdownRenderer::new(max_width);

    for event in parser {
        renderer.handle_event(event);
    }

    renderer.finish()
}

struct TuiMarkdownRenderer {
    lines: Vec<StyledLine>,
    current_line: Vec<(String, LineStyle)>,
    max_width: usize,
    current_width: usize,
    style_stack: Vec<MarkdownStyle>,
    list_stack: Vec<ListContext>,
    in_code_block: bool,
    code_block_content: String,
    in_list_item_start: bool, // Suppress paragraph blank line right after list bullet
}

#[derive(Clone)]
enum MarkdownStyle {
    Bold,
    Italic,
    Strikethrough,
    Quote,
    Link,
    Heading,
}

#[derive(Clone)]
struct ListContext {
    index: Option<u64>,
    depth: usize,
}

impl TuiMarkdownRenderer {
    fn new(max_width: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_line: Vec::new(),
            max_width,
            current_width: 0,
            style_stack: Vec::new(),
            list_stack: Vec::new(),
            in_code_block: false,
            code_block_content: String::new(),
            in_list_item_start: false,
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => self.inline_code(&code),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => self.rule(),
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                // Don't add blank line if we just started a list item (bullet is on same line)
                if !self.in_list_item_start
                    && (!self.lines.is_empty() || !self.current_line.is_empty())
                {
                    self.ensure_blank_line();
                }
                self.in_list_item_start = false;
            }
            Tag::Heading { level, .. } => {
                self.ensure_blank_line();
                let depth = heading_level_to_usize(level);
                let prefix = format!("{} ", "#".repeat(depth));
                self.push_styled_text(
                    &prefix,
                    LineStyle {
                        fg: Some(CYAN),
                        bold: true,
                        ..Default::default()
                    },
                );
                self.style_stack.push(MarkdownStyle::Heading);
            }
            Tag::CodeBlock(kind) => {
                self.ensure_blank_line();
                self.in_code_block = true;
                self.code_block_content.clear();
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let fence = if lang.is_empty() {
                    "```".to_string()
                } else {
                    format!("```{}", lang)
                };
                self.push_styled_text(
                    &fence,
                    LineStyle {
                        dimmed: true,
                        ..Default::default()
                    },
                );
                self.flush_line();
            }
            Tag::List(start) => {
                // Add blank line before top-level lists only
                if self.list_stack.is_empty() {
                    self.ensure_blank_line();
                } else {
                    self.flush_line();
                }
                let depth = self.list_stack.len();
                self.list_stack.push(ListContext {
                    index: start,
                    depth,
                });
            }
            Tag::Item => {
                self.flush_line();
                // Extract values from list context before calling methods
                let (indent, bullet) = if let Some(ctx) = self.list_stack.last_mut() {
                    let indent = "  ".repeat(ctx.depth);
                    let bullet = match &mut ctx.index {
                        None => (format!("{}- ", indent), false),
                        Some(n) => {
                            let b = format!("{}{}. ", indent, n);
                            *n += 1;
                            (b, true)
                        }
                    };
                    (Some(indent), Some(bullet))
                } else {
                    (None, None)
                };
                if let Some((text, is_numbered)) = bullet {
                    let style = if is_numbered {
                        LineStyle {
                            dimmed: true,
                            ..Default::default()
                        }
                    } else {
                        LineStyle::default()
                    };
                    self.push_styled_text(&text, style);
                }
                let _ = indent; // Mark as intentionally unused
                self.in_list_item_start = true; // Next paragraph shouldn't add blank line
            }
            Tag::Emphasis => self.style_stack.push(MarkdownStyle::Italic),
            Tag::Strong => self.style_stack.push(MarkdownStyle::Bold),
            Tag::Strikethrough => self.style_stack.push(MarkdownStyle::Strikethrough),
            Tag::BlockQuote(_) => {
                self.ensure_blank_line();
                self.push_styled_text(
                    "> ",
                    LineStyle {
                        fg: Some(GREEN),
                        ..Default::default()
                    },
                );
                self.style_stack.push(MarkdownStyle::Quote);
            }
            Tag::Link { .. } => {
                self.style_stack.push(MarkdownStyle::Link);
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line();
            }
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.flush_line();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                // Take ownership of code block content to avoid borrow issues
                let code_content = std::mem::take(&mut self.code_block_content);
                // Output code block content
                for code_line in code_content.lines() {
                    self.push_styled_text(
                        code_line,
                        LineStyle {
                            fg: Some(CODE_COLOR),
                            ..Default::default()
                        },
                    );
                    self.flush_line();
                }
                self.push_styled_text(
                    "```",
                    LineStyle {
                        dimmed: true,
                        ..Default::default()
                    },
                );
                self.flush_line();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                self.in_list_item_start = false; // Clear flag when list ends
            }
            TagEnd::Item => {
                self.flush_line();
                self.in_list_item_start = false; // Clear flag when item ends
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.style_stack.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.style_stack.pop();
                self.flush_line();
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_block_content.push_str(text);
            return;
        }

        let style = self.current_style();

        // Handle text wrapping
        for word in text.split_inclusive(char::is_whitespace) {
            let word_width = word.chars().count();

            // Check if we need to wrap
            if self.current_width + word_width > self.max_width && self.current_width > 0 {
                self.flush_line();
                // Add list indent on continuation
                if let Some(ctx) = self.list_stack.last() {
                    let indent = "  ".repeat(ctx.depth + 1);
                    self.push_styled_text(&indent, LineStyle::default());
                }
            }

            self.push_styled_text(word, style.clone());
        }
    }

    fn inline_code(&mut self, code: &str) {
        self.push_styled_text(
            code,
            LineStyle {
                fg: Some(CODE_COLOR),
                ..Default::default()
            },
        );
    }

    fn soft_break(&mut self) {
        // Preserve line breaks
        self.flush_line();
    }

    fn hard_break(&mut self) {
        self.flush_line();
    }

    fn rule(&mut self) {
        self.ensure_blank_line();
        let rule = "─".repeat(self.max_width.min(40));
        self.push_styled_text(
            &rule,
            LineStyle {
                dimmed: true,
                ..Default::default()
            },
        );
        self.flush_line();
    }

    fn push_styled_text(&mut self, text: &str, style: LineStyle) {
        if !text.is_empty() {
            self.current_line.push((text.to_string(), style));
            self.current_width += text.chars().count();
        }
    }

    fn flush_line(&mut self) {
        if !self.current_line.is_empty() {
            self.lines.push(StyledLine {
                spans: std::mem::take(&mut self.current_line),
            });
        }
        self.current_width = 0;
    }

    fn ensure_blank_line(&mut self) {
        self.flush_line();
        if self.lines.last().is_some_and(|l| !l.spans.is_empty()) {
            self.lines.push(StyledLine { spans: vec![] });
        }
    }

    fn current_style(&self) -> LineStyle {
        let mut style = LineStyle::default();

        for s in &self.style_stack {
            match s {
                MarkdownStyle::Bold => style.bold = true,
                MarkdownStyle::Italic => {
                    // Ratatui doesn't have italic, use a color hint
                    if style.fg.is_none() {
                        style.fg = Some((200, 200, 200));
                    }
                }
                MarkdownStyle::Strikethrough => style.dimmed = true,
                MarkdownStyle::Quote => style.fg = Some(GREEN),
                MarkdownStyle::Link => style.fg = Some(BLUE),
                MarkdownStyle::Heading => {
                    style.fg = Some(CYAN);
                    style.bold = true;
                }
            }
        }

        style
    }

    fn finish(mut self) -> Vec<StyledLine> {
        self.flush_line();
        // Remove trailing empty lines
        while self.lines.last().is_some_and(|l| l.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

fn heading_level_to_usize(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Apply italic and dimmed styling to thinking block content
fn apply_thinking_style(styled_lines: Vec<StyledLine>) -> Vec<StyledLine> {
    styled_lines
        .into_iter()
        .map(|line| StyledLine {
            spans: line
                .spans
                .into_iter()
                .map(|(text, mut style)| {
                    style.italic = true;
                    style.fg = Some(THINKING_TEXT);
                    (text, style)
                })
                .collect(),
        })
        .collect()
}

/// Render ledger block with styled markdown lines
fn render_ledger_block_styled(
    lines: &mut Vec<RenderedLine>,
    name: &str,
    color: (u8, u8, u8),
    bold: bool,
    styled_lines: Vec<StyledLine>,
) {
    for (i, styled_line) in styled_lines.iter().enumerate() {
        let mut spans = Vec::new();

        // Name column (right-aligned, only on first line)
        let name_text = if i == 0 {
            format!("{:>width$}", name, width = NAME_WIDTH)
        } else {
            " ".repeat(NAME_WIDTH)
        };

        spans.push((
            name_text,
            LineStyle {
                fg: Some(color),
                bold,
                dimmed: false,
                italic: false,
            },
        ));

        // Separator
        spans.push((
            " │ ".to_string(),
            LineStyle {
                fg: Some(SEPARATOR_COLOR),
                ..Default::default()
            },
        ));

        // Content spans
        if styled_line.spans.is_empty() {
            // Empty line - just push name and separator
        } else {
            for (text, style) in &styled_line.spans {
                spans.push((text.clone(), style.clone()));
            }
        }

        lines.push(RenderedLine { spans });
    }

    // If no lines, still output at least the name
    if styled_lines.is_empty() {
        let spans = vec![
            (
                format!("{:>width$}", name, width = NAME_WIDTH),
                LineStyle {
                    fg: Some(color),
                    bold,
                    dimmed: false,
                    italic: false,
                },
            ),
            (
                " │ ".to_string(),
                LineStyle {
                    fg: Some(SEPARATOR_COLOR),
                    ..Default::default()
                },
            ),
        ];
        lines.push(RenderedLine { spans });
    }
}

/// Render ledger block with plain text (no markdown)
fn render_ledger_block_plain(
    lines: &mut Vec<RenderedLine>,
    name: &str,
    color: (u8, u8, u8),
    bold: bool,
    text: &str,
) {
    for (i, line_text) in text.lines().enumerate() {
        let mut spans = Vec::new();

        // Name column (right-aligned, only on first line)
        let name_text = if i == 0 {
            format!("{:>width$}", name, width = NAME_WIDTH)
        } else {
            " ".repeat(NAME_WIDTH)
        };

        spans.push((
            name_text,
            LineStyle {
                fg: Some(color),
                bold,
                dimmed: false,
                italic: false,
            },
        ));

        // Separator
        spans.push((
            " │ ".to_string(),
            LineStyle {
                fg: Some(SEPARATOR_COLOR),
                ..Default::default()
            },
        ));

        // Content
        spans.push((line_text.to_string(), LineStyle::default()));

        lines.push(RenderedLine { spans });
    }
}

fn render_continuation(lines: &mut Vec<RenderedLine>, text: &str) {
    for line_text in text.lines() {
        let spans = vec![
            (" ".repeat(NAME_WIDTH), LineStyle::default()),
            (
                " │ ".to_string(),
                LineStyle {
                    fg: Some(SEPARATOR_COLOR),
                    ..Default::default()
                },
            ),
            (
                line_text.to_string(),
                LineStyle {
                    dimmed: true,
                    ..Default::default()
                },
            ),
        ];

        lines.push(RenderedLine { spans });
    }
}

/// Process user message text to handle command-related XML tags.
/// Returns None if the message should be skipped entirely (e.g., empty local-command-stdout).
fn process_command_message(text: &str) -> Option<String> {
    let trimmed = text.trim();

    // Check for empty or whitespace-only local-command-stdout - skip these entirely
    if trimmed.starts_with("<local-command-stdout>") && trimmed.ends_with("</local-command-stdout>")
    {
        let tag_start = "<local-command-stdout>".len();
        let tag_end = trimmed.len() - "</local-command-stdout>".len();
        let inner = &trimmed[tag_start..tag_end];
        if inner.trim().is_empty() {
            return None;
        }
        // Non-empty local-command-stdout: show the content without the tags
        return Some(inner.trim().to_string());
    }

    // Check if this is a command message with <command-name> tag
    if let Some(start) = trimmed.find("<command-name>")
        && let Some(end) = trimmed.find("</command-name>")
    {
        let content_start = start + "<command-name>".len();
        if content_start < end {
            return Some(trimmed[content_start..end].to_string());
        }
    }

    Some(text.to_string())
}
