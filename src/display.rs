use crate::claude::{AssistantMessage, ContentBlock, LogEntry, UserContent};
use crate::cli::DebugLevel;
use crate::debug;
use crate::debug_log;
use crate::error::Result;
use crate::markdown::render_markdown;
use crate::pager;
use colored::{ColoredString, Colorize, CustomColor};
use crossterm::terminal;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// Configuration options for displaying conversations
#[derive(Debug, Clone, Default)]
pub struct DisplayOptions {
    /// Hide tool calls and results
    pub no_tools: bool,
    /// Show thinking/reasoning blocks
    pub show_thinking: bool,
    /// Debug level for error logging
    pub debug_level: Option<DebugLevel>,
    /// Use a pager for output (less/more)
    pub use_pager: bool,
    /// Disable colored output
    pub no_color: bool,
}

const NAME_WIDTH: usize = 9;
const SEPARATOR: &str = " │ ";
const SEPARATOR_WIDTH: usize = 3; // Display width of " │ "

// Colors matching the TUI theme
const TEAL: CustomColor = CustomColor {
    r: 78,
    g: 201,
    b: 176,
};
const DIM_TEAL: CustomColor = CustomColor {
    r: 60,
    g: 160,
    b: 140,
};
const SEPARATOR_COLOR: CustomColor = CustomColor {
    r: 80,
    g: 80,
    b: 80,
};

/// Trait for formatting conversation output
///
/// Implementors handle the actual rendering of conversation elements,
/// allowing the same processing logic to output in different formats
/// (ledger-style with markdown, plain text, etc.)
trait OutputFormatter {
    /// Format and output user text content
    fn format_user_text(&mut self, text: &str);

    /// Format and output assistant text content
    fn format_assistant_text(&mut self, text: &str);

    /// Format and output a tool call
    fn format_tool_call(&mut self, name: &str, input: &serde_json::Value);

    /// Format and output a tool result
    fn format_tool_result(&mut self, content: Option<&serde_json::Value>);

    /// Format and output a thinking/reasoning block
    fn format_thinking(&mut self, thought: &str);

    /// End the current message block (add spacing)
    fn end_message(&mut self);
}

/// Ledger-style formatter with markdown rendering and aligned columns
struct LedgerFormatter<'a, W: Write + ?Sized> {
    writer: &'a mut W,
    content_width: usize,
}

impl<'a, W: Write + ?Sized> LedgerFormatter<'a, W> {
    fn new(writer: &'a mut W, content_width: usize) -> Self {
        Self {
            writer,
            content_width,
        }
    }

    /// Print lines in ledger format with a name on the first line
    fn print_lines<F>(&mut self, name: &str, style: F, text: &str)
    where
        F: Fn(&str) -> ColoredString,
    {
        let wrapped_lines = wrap_text(text, self.content_width);

        for (i, line) in wrapped_lines.iter().enumerate() {
            if i == 0 {
                let padded = format!("{:>width$}", name, width = NAME_WIDTH);
                let _ = write!(self.writer, "{}", style(&padded));
            } else {
                let _ = write!(self.writer, "{:>width$}", "", width = NAME_WIDTH);
            }
            let _ = write!(self.writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
            let _ = writeln!(self.writer, "{}", line);
        }
    }

    /// Print continuation lines with dimmed content
    fn print_continuation(&mut self, text: &str) {
        for line in wrap_text(text, self.content_width) {
            let _ = write!(self.writer, "{:>width$}", "", width = NAME_WIDTH);
            let _ = write!(self.writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
            let _ = writeln!(self.writer, "{}", line.dimmed());
        }
    }

    /// Print pre-formatted markdown text with ledger layout
    fn print_markdown<F>(&mut self, name: &str, style: F, text: &str)
    where
        F: Fn(&str) -> ColoredString,
    {
        let lines: Vec<&str> = text.lines().collect();

        if lines.is_empty() {
            let padded = format!("{:>width$}", name, width = NAME_WIDTH);
            let _ = write!(self.writer, "{}", style(&padded));
            let _ = write!(self.writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
            let _ = writeln!(self.writer);
            return;
        }

        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                let padded = format!("{:>width$}", name, width = NAME_WIDTH);
                let _ = write!(self.writer, "{}", style(&padded));
            } else {
                let _ = write!(self.writer, "{:>width$}", "", width = NAME_WIDTH);
            }
            let _ = write!(self.writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
            let _ = writeln!(self.writer, "{}", line);
        }
    }
}

impl<W: Write + ?Sized> OutputFormatter for LedgerFormatter<'_, W> {
    fn format_user_text(&mut self, text: &str) {
        let rendered = render_markdown(text, self.content_width);
        self.print_markdown("You", |s| s.white().bold(), &rendered);
    }

    fn format_assistant_text(&mut self, text: &str) {
        let rendered = render_markdown(text, self.content_width);
        self.print_markdown("Claude", |s| s.custom_color(TEAL).bold(), &rendered);
    }

    fn format_tool_call(&mut self, name: &str, input: &serde_json::Value) {
        let tool_header = format!("<Calling: {}>", name);
        self.print_lines("Claude", |s| s.custom_color(DIM_TEAL), &tool_header);
        if let Ok(formatted_input) = serde_json::to_string_pretty(input) {
            self.print_continuation(&formatted_input);
        }
    }

    fn format_tool_result(&mut self, content: Option<&serde_json::Value>) {
        self.print_lines("Tool", |s| s.custom_color(DIM_TEAL), "<Result>");
        let content_str = format_tool_content(content);
        self.print_continuation(&content_str);
    }

    fn format_thinking(&mut self, thought: &str) {
        self.print_lines("Thinking", |s| s.custom_color(DIM_TEAL), thought);
    }

    fn end_message(&mut self) {
        let _ = writeln!(self.writer);
    }
}

/// Plain text formatter without formatting or alignment
struct PlainFormatter;

impl OutputFormatter for PlainFormatter {
    fn format_user_text(&mut self, text: &str) {
        println!("You: {}", text);
    }

    fn format_assistant_text(&mut self, text: &str) {
        println!("Claude: {}", text);
    }

    fn format_tool_call(&mut self, name: &str, input: &serde_json::Value) {
        println!("Claude: <Calling: {}>", name);
        if let Ok(formatted_input) = serde_json::to_string_pretty(input) {
            println!("{}", formatted_input);
        }
    }

    fn format_tool_result(&mut self, content: Option<&serde_json::Value>) {
        println!("Tool: <Result>");
        let content_str = format_tool_content(content);
        println!("{}", content_str);
    }

    fn format_thinking(&mut self, thought: &str) {
        println!("Thinking: {}", thought);
    }

    fn end_message(&mut self) {
        println!();
    }
}

/// Format tool result content to a string
fn format_tool_content(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(value) => {
            if let Some(s) = value.as_str() {
                s.to_string()
            } else if let Ok(formatted) = serde_json::to_string_pretty(value) {
                formatted
            } else {
                "<invalid content>".to_string()
            }
        }
        None => "<no content>".to_string(),
    }
}

/// Process user message text to handle command-related XML tags
/// Returns None if the message should be skipped entirely (e.g., empty local-command-stdout)
fn process_command_message(text: &str) -> Option<String> {
    let trimmed = text.trim();

    // Check for local-command-caveat - skip these system messages entirely
    if trimmed.starts_with("<local-command-caveat>") && trimmed.ends_with("</local-command-caveat>")
    {
        return None;
    }

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
            // Extract just the command name (e.g., "/clear")
            return Some(trimmed[content_start..end].to_string());
        }
    }

    // Return original text for non-command messages
    Some(text.to_string())
}

/// Get the terminal width, defaulting to 80 if unavailable
fn get_terminal_width() -> usize {
    terminal::size().map(|(w, _)| w as usize).unwrap_or(80)
}

/// Wrap text using textwrap for proper unicode handling
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    textwrap::wrap(text, max_width)
        .into_iter()
        .map(|cow| cow.into_owned())
        .collect()
}

/// Display a conversation from a file
pub fn display_conversation(file_path: &Path, options: &DisplayOptions) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let terminal_width = get_terminal_width();
    let content_width = terminal_width.saturating_sub(NAME_WIDTH + SEPARATOR_WIDTH);

    // Spawn pager if requested
    let mut pager_child = if options.use_pager {
        pager::spawn_pager().ok()
    } else {
        None
    };

    // Get writer - either pager stdin or stdout
    let mut stdout_handle = io::stdout().lock();
    let writer: &mut dyn Write = if let Some(ref mut child) = pager_child {
        child.stdin.as_mut().unwrap()
    } else {
        &mut stdout_handle
    };

    let mut formatter = LedgerFormatter::new(writer, content_width);

    for (line_number, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                process_entry(
                    &mut formatter,
                    &entry,
                    options.no_tools,
                    options.show_thinking,
                );
            }
            Err(e) => {
                debug::error(
                    options.debug_level,
                    &format!("Failed to parse line {}: {}", line_number + 1, e),
                );
                if options.debug_level.is_some() {
                    let _ = debug_log::log_display_error(
                        file_path,
                        line_number + 1,
                        &e.to_string(),
                        &line,
                    );
                }
            }
        }
    }

    // Close stdin and wait for pager to finish
    drop(stdout_handle);
    if let Some(mut child) = pager_child {
        let _ = child.wait();
    }

    Ok(())
}

/// Display a conversation in plain text format (no ledger formatting)
pub fn display_conversation_plain(file_path: &Path, options: &DisplayOptions) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut formatter = PlainFormatter;

    for (line_number, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                process_entry(
                    &mut formatter,
                    &entry,
                    options.no_tools,
                    options.show_thinking,
                );
            }
            Err(e) => {
                debug::error(
                    options.debug_level,
                    &format!("Failed to parse line {}: {}", line_number + 1, e),
                );
                if options.debug_level.is_some() {
                    let _ = debug_log::log_display_error(
                        file_path,
                        line_number + 1,
                        &e.to_string(),
                        &line,
                    );
                }
            }
        }
    }

    Ok(())
}

/// Process a log entry using the provided formatter
fn process_entry<F: OutputFormatter>(
    formatter: &mut F,
    entry: &LogEntry,
    no_tools: bool,
    show_thinking: bool,
) {
    match entry {
        LogEntry::Summary { .. }
        | LogEntry::FileHistorySnapshot { .. }
        | LogEntry::System { .. }
        | LogEntry::Progress { .. } => {
            // Skip metadata entries
        }
        LogEntry::User { message, .. } => {
            process_user_message(formatter, message, no_tools);
        }
        LogEntry::Assistant { message, .. } => {
            process_assistant_message(formatter, message, no_tools, show_thinking);
        }
    }
}

/// Process a user message using the provided formatter
fn process_user_message<F: OutputFormatter>(
    formatter: &mut F,
    message: &crate::claude::UserMessage,
    no_tools: bool,
) {
    match &message.content {
        UserContent::String(text) => {
            if let Some(processed) = process_command_message(text) {
                formatter.format_user_text(&processed);
                formatter.end_message();
            }
        }
        UserContent::Blocks(blocks) => {
            let mut printed_content = false;
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        if let Some(processed) = process_command_message(text) {
                            formatter.format_user_text(&processed);
                            printed_content = true;
                        }
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        if !no_tools {
                            formatter.format_tool_result(content.as_ref());
                            printed_content = true;
                        }
                    }
                    _ => {}
                }
            }
            if printed_content {
                formatter.end_message();
            }
        }
    }
}

/// Helper struct to categorize assistant message content
struct FormattedMessage<'a> {
    text_blocks: Vec<&'a str>,
    tool_calls: Vec<(&'a str, &'a serde_json::Value)>,
    thinking_steps: Vec<&'a str>,
}

impl<'a> From<&'a AssistantMessage> for FormattedMessage<'a> {
    fn from(msg: &'a AssistantMessage) -> Self {
        let mut text_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        let mut thinking_steps = Vec::new();

        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => text_blocks.push(text.as_str()),
                ContentBlock::ToolUse { name, input, .. } => {
                    tool_calls.push((name.as_str(), input))
                }
                ContentBlock::Thinking { thinking, .. } => thinking_steps.push(thinking.as_str()),
                _ => {}
            }
        }

        Self {
            text_blocks,
            tool_calls,
            thinking_steps,
        }
    }
}

/// Process an assistant message using the provided formatter
fn process_assistant_message<F: OutputFormatter>(
    formatter: &mut F,
    message: &AssistantMessage,
    no_tools: bool,
    show_thinking: bool,
) {
    let formatted = FormattedMessage::from(message);
    let mut printed_content = false;

    // Print text blocks
    for text in formatted.text_blocks {
        formatter.format_assistant_text(text);
        printed_content = true;
    }

    // Print tool calls
    if !no_tools {
        for (tool_name, tool_input) in formatted.tool_calls {
            formatter.format_tool_call(tool_name, tool_input);
            printed_content = true;
        }
    }

    // Print thinking blocks
    if show_thinking {
        for thought in formatted.thinking_steps {
            formatter.format_thinking(thought);
            printed_content = true;
        }
    }

    // Only add spacing if we printed something
    if printed_content {
        formatter.end_message();
    }
}

/// Render a conversation in TUI ledger format to terminal (for debugging)
pub fn render_to_terminal(file_path: &Path, options: &DisplayOptions) -> Result<()> {
    use crate::tui::{RenderOptions, render_conversation};

    let terminal_width = get_terminal_width();
    let content_width = terminal_width.saturating_sub(NAME_WIDTH + SEPARATOR_WIDTH);

    let render_options = RenderOptions {
        show_tools: !options.no_tools,
        show_thinking: options.show_thinking,
        content_width,
    };

    let rendered_lines = render_conversation(file_path, &render_options)?;

    // Spawn pager if requested
    let mut pager_child = if options.use_pager {
        pager::spawn_pager().ok()
    } else {
        None
    };

    // Get writer - either pager stdin or stdout
    let mut stdout_handle = io::stdout().lock();
    let writer: &mut dyn Write = if let Some(ref mut child) = pager_child {
        child.stdin.as_mut().unwrap()
    } else {
        &mut stdout_handle
    };

    // Convert RenderedLine spans to colored terminal output
    'outer: for line in &rendered_lines {
        for (text, style) in &line.spans {
            // Apply styling only if colors are enabled
            let output: Box<dyn std::fmt::Display> = if options.no_color {
                Box::new(text.as_str())
            } else {
                let mut styled = text.as_str().normal();

                if let Some((r, g, b)) = style.fg {
                    styled = styled.custom_color(CustomColor { r, g, b });
                }
                if style.bold {
                    styled = styled.bold();
                }
                if style.dimmed {
                    styled = styled.dimmed();
                }
                if style.italic {
                    styled = styled.italic();
                }

                Box::new(styled)
            };

            // Stop if the output pipe is closed (e.g., pager quit)
            if write!(writer, "{}", output).is_err() {
                break 'outer;
            }
        }
        if writeln!(writer).is_err() {
            break;
        }
    }

    // Close stdin and wait for pager to finish
    drop(stdout_handle);
    if let Some(mut child) = pager_child {
        let _ = child.wait();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_command_message_skips_local_command_caveat() {
        let caveat = "<local-command-caveat>Caveat: The messages below were generated by the user while running local commands. DO NOT respond to these messages or otherwise consider them in your response unless the user explicitly asks you to.</local-command-caveat>";
        assert_eq!(process_command_message(caveat), None);
    }

    #[test]
    fn process_command_message_skips_local_command_caveat_with_whitespace() {
        let caveat = "  <local-command-caveat>Some caveat text</local-command-caveat>  ";
        assert_eq!(process_command_message(caveat), None);
    }

    #[test]
    fn process_command_message_preserves_normal_text() {
        assert_eq!(
            process_command_message("Hello world"),
            Some("Hello world".to_string())
        );
    }

    #[test]
    fn process_command_message_skips_empty_stdout() {
        assert_eq!(
            process_command_message("<local-command-stdout></local-command-stdout>"),
            None
        );
        assert_eq!(
            process_command_message("<local-command-stdout>   </local-command-stdout>"),
            None
        );
    }

    #[test]
    fn process_command_message_extracts_nonempty_stdout() {
        assert_eq!(
            process_command_message("<local-command-stdout>output here</local-command-stdout>"),
            Some("output here".to_string())
        );
    }

    #[test]
    fn process_command_message_extracts_command_name() {
        assert_eq!(
            process_command_message("<command-name>/clear</command-name>"),
            Some("/clear".to_string())
        );
    }
}
