use crate::claude::{AssistantMessage, ContentBlock, LogEntry, UserContent};
use crate::cli::DebugLevel;
use crate::debug;
use crate::debug_log;
use crate::error::Result;
use crate::pager;
use colored::{ColoredString, Colorize, CustomColor};
use crossterm::terminal;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

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

/// Process user message text to handle command-related XML tags
/// Returns None if the message should be skipped entirely (e.g., empty local-command-stdout)
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

/// Print lines in ledger format with a name on the first line and blank name column for continuations
fn print_ledger_lines<W, F>(writer: &mut W, name: &str, style: F, text: &str, content_width: usize)
where
    W: Write + ?Sized,
    F: Fn(&str) -> ColoredString,
{
    let wrapped_lines = wrap_text(text, content_width);

    for (i, line) in wrapped_lines.iter().enumerate() {
        if i == 0 {
            // Pad the plain text first, then apply color to avoid ANSI escape code interference
            let padded = format!("{:>width$}", name, width = NAME_WIDTH);
            let _ = write!(writer, "{}", style(&padded));
        } else {
            let _ = write!(writer, "{:>width$}", "", width = NAME_WIDTH);
        }
        let _ = write!(writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
        let _ = writeln!(writer, "{}", line);
    }
}

/// Print continuation lines (for tool output, etc.) with dimmed content
fn print_ledger_continuation<W: Write + ?Sized>(writer: &mut W, text: &str, content_width: usize) {
    for line in wrap_text(text, content_width) {
        let _ = write!(writer, "{:>width$}", "", width = NAME_WIDTH);
        let _ = write!(writer, "{}", SEPARATOR.custom_color(SEPARATOR_COLOR));
        let _ = writeln!(writer, "{}", line.dimmed());
    }
}

/// Display a conversation from a file
pub fn display_conversation(
    file_path: &Path,
    no_tools: bool,
    show_thinking: bool,
    debug_level: Option<DebugLevel>,
    use_pager: bool,
) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let terminal_width = get_terminal_width();
    let content_width = terminal_width.saturating_sub(NAME_WIDTH + SEPARATOR_WIDTH);

    // Spawn pager if requested
    let mut pager_child = if use_pager {
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

    for (line_number, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                display_entry(writer, &entry, no_tools, show_thinking, content_width);
            }
            Err(e) => {
                debug::error(
                    debug_level,
                    &format!("Failed to parse line {}: {}", line_number + 1, e),
                );
                if debug_level.is_some() {
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

fn display_entry<W: Write + ?Sized>(
    writer: &mut W,
    entry: &LogEntry,
    no_tools: bool,
    show_thinking: bool,
    content_width: usize,
) {
    match entry {
        LogEntry::Summary { .. }
        | LogEntry::FileHistorySnapshot { .. }
        | LogEntry::System { .. }
        | LogEntry::Progress { .. } => {
            // Skip metadata entries
        }
        LogEntry::User { message, .. } => match &message.content {
            UserContent::String(text) => {
                if let Some(processed) = process_command_message(text) {
                    print_ledger_lines(
                        writer,
                        "You",
                        |s| s.white().bold(),
                        &processed,
                        content_width,
                    );
                    let _ = writeln!(writer);
                }
            }
            UserContent::Blocks(blocks) => {
                let mut printed_content = false;
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => {
                            if let Some(processed) = process_command_message(text) {
                                print_ledger_lines(
                                    writer,
                                    "You",
                                    |s| s.white().bold(),
                                    &processed,
                                    content_width,
                                );
                                printed_content = true;
                            }
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            if !no_tools {
                                print_ledger_lines(
                                    writer,
                                    "Tool",
                                    |s| s.custom_color(DIM_TEAL),
                                    "<Result>",
                                    content_width,
                                );
                                if let Some(content_value) = content {
                                    let content_str =
                                        if let Some(result_str) = content_value.as_str() {
                                            result_str.to_string()
                                        } else if let Ok(formatted_result) =
                                            serde_json::to_string_pretty(content_value)
                                        {
                                            formatted_result
                                        } else {
                                            "<invalid content>".to_string()
                                        };
                                    print_ledger_continuation(writer, &content_str, content_width);
                                } else {
                                    print_ledger_continuation(
                                        writer,
                                        "<no content>",
                                        content_width,
                                    );
                                }
                                printed_content = true;
                            }
                        }
                        _ => {}
                    }
                }
                if printed_content {
                    let _ = writeln!(writer);
                }
            }
        },
        LogEntry::Assistant { message, .. } => {
            display_assistant_message(writer, message, no_tools, show_thinking, content_width);
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

fn display_assistant_message<W: Write + ?Sized>(
    writer: &mut W,
    message: &AssistantMessage,
    no_tools: bool,
    show_thinking: bool,
    content_width: usize,
) {
    let formatted = FormattedMessage::from(message);
    let mut printed_content = false;

    // Print text blocks
    for text in formatted.text_blocks {
        print_ledger_lines(
            writer,
            "Claude",
            |s| s.custom_color(TEAL).bold(),
            text,
            content_width,
        );
        printed_content = true;
    }

    // Print tool calls
    if !no_tools {
        for (tool_name, tool_input) in formatted.tool_calls {
            let tool_header = format!("<Calling: {}>", tool_name);
            print_ledger_lines(
                writer,
                "Claude",
                |s| s.custom_color(DIM_TEAL),
                &tool_header,
                content_width,
            );
            if let Ok(formatted_input) = serde_json::to_string_pretty(tool_input) {
                print_ledger_continuation(writer, &formatted_input, content_width);
            }
            printed_content = true;
        }
    }

    // Print thinking blocks
    if show_thinking {
        for thought in formatted.thinking_steps {
            print_ledger_lines(
                writer,
                "Thinking",
                |s| s.custom_color(DIM_TEAL),
                thought,
                content_width,
            );
            printed_content = true;
        }
    }

    // Only add blank line separator if we printed something
    if printed_content {
        let _ = writeln!(writer);
    }
}

/// Display a conversation in plain text format (no ledger formatting)
pub fn display_conversation_plain(
    file_path: &Path,
    no_tools: bool,
    show_thinking: bool,
    debug_level: Option<DebugLevel>,
) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    for (line_number, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                display_entry_plain(&entry, no_tools, show_thinking);
            }
            Err(e) => {
                debug::error(
                    debug_level,
                    &format!("Failed to parse line {}: {}", line_number + 1, e),
                );
                if debug_level.is_some() {
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

fn display_entry_plain(entry: &LogEntry, no_tools: bool, show_thinking: bool) {
    match entry {
        LogEntry::Summary { .. }
        | LogEntry::FileHistorySnapshot { .. }
        | LogEntry::System { .. }
        | LogEntry::Progress { .. } => {
            // Skip metadata entries
        }
        LogEntry::User { message, .. } => match &message.content {
            UserContent::String(text) => {
                if let Some(processed) = process_command_message(text) {
                    println!("You: {}", processed);
                    println!();
                }
            }
            UserContent::Blocks(blocks) => {
                let mut printed_content = false;
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => {
                            if let Some(processed) = process_command_message(text) {
                                println!("You: {}", processed);
                                printed_content = true;
                            }
                        }
                        ContentBlock::ToolResult { content, .. } => {
                            if !no_tools {
                                println!("Tool: <Result>");
                                if let Some(content_value) = content {
                                    let content_str =
                                        if let Some(result_str) = content_value.as_str() {
                                            result_str.to_string()
                                        } else if let Ok(formatted_result) =
                                            serde_json::to_string_pretty(content_value)
                                        {
                                            formatted_result
                                        } else {
                                            "<invalid content>".to_string()
                                        };
                                    println!("{}", content_str);
                                } else {
                                    println!("<no content>");
                                }
                                printed_content = true;
                            }
                        }
                        _ => {}
                    }
                }
                if printed_content {
                    println!();
                }
            }
        },
        LogEntry::Assistant { message, .. } => {
            display_assistant_message_plain(message, no_tools, show_thinking);
        }
    }
}

fn display_assistant_message_plain(
    message: &AssistantMessage,
    no_tools: bool,
    show_thinking: bool,
) {
    let formatted = FormattedMessage::from(message);
    let mut printed_content = false;

    // Print text blocks
    for text in formatted.text_blocks {
        println!("Claude: {}", text);
        printed_content = true;
    }

    // Print tool calls
    if !no_tools {
        for (tool_name, tool_input) in formatted.tool_calls {
            println!("Claude: <Calling: {}>", tool_name);
            if let Ok(formatted_input) = serde_json::to_string_pretty(tool_input) {
                println!("{}", formatted_input);
            }
            printed_content = true;
        }
    }

    // Print thinking blocks
    if show_thinking {
        for thought in formatted.thinking_steps {
            println!("Thinking: {}", thought);
            printed_content = true;
        }
    }

    if printed_content {
        println!();
    }
}
