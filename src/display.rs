use crate::claude::{AssistantMessage, ContentBlock, LogEntry, UserContent};
use crate::error::Result;
use colored::*;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Display a conversation from a file
pub fn display_conversation(file_path: &Path, no_tools: bool, show_thinking: bool) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    for line_result in reader.lines() {
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                display_entry(&entry, no_tools, show_thinking);
            }
            Err(e) => {
                eprintln!("Failed to parse line: {}", e);
                eprintln!("Line content: {}", line);
            }
        }
    }

    Ok(())
}

fn display_entry(entry: &LogEntry, no_tools: bool, show_thinking: bool) {
    match entry {
        LogEntry::Summary { .. }
        | LogEntry::FileHistorySnapshot { .. }
        | LogEntry::System { .. } => {
            // Skip summary, file history snapshot, and system entries
        }
        LogEntry::User { message, .. } => match &message.content {
            UserContent::String(text) => {
                println!("{} {}", "User:".blue().bold(), text);
            }
            UserContent::Blocks(blocks) => {
                for block in blocks {
                    if let ContentBlock::Text { text } = block {
                        println!("{} {}", "User:".blue().bold(), text);
                    }
                }
            }
        },
        LogEntry::Assistant { message, .. } => {
            display_assistant_message(message, no_tools, show_thinking);
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

fn display_assistant_message(message: &AssistantMessage, no_tools: bool, show_thinking: bool) {
    let formatted = FormattedMessage::from(message);

    for text in formatted.text_blocks {
        println!("{} {}", "Assistant:".green().bold(), text);
    }

    if !no_tools {
        for (tool_name, tool_input) in formatted.tool_calls {
            println!(
                "{} <Calling Tool: {}>",
                "Assistant:".green().bold(),
                tool_name
            );
            if let Ok(formatted_input) = serde_json::to_string_pretty(tool_input) {
                println!("{}", formatted_input.dimmed());
            }
        }
    }

    if show_thinking {
        for thought in formatted.thinking_steps {
            println!("{} {}", "Thinking:".yellow().bold(), thought);
        }
    }
}
