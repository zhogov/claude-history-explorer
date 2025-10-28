use chrono::{DateTime, Local};
use clap::Parser;
use colored::*;
use serde::Deserialize;
use std::fs::{read_dir, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
#[command(name = "claude-history-viewer")]
#[command(about = "View Claude conversation history with fuzzy search")]
struct Args {
    /// Hide tool calls from the output
    #[arg(long, help = "Hide tool calls from the conversation output")]
    no_tools: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
enum LogEntry {
    Summary {
        #[allow(dead_code)]
        summary: String,
    },
    User {
        message: UserMessage,
        #[allow(dead_code)]
        timestamp: String,
    },
    Assistant {
        message: AssistantMessage,
        #[allow(dead_code)]
        timestamp: String,
    },
    #[serde(rename = "file-history-snapshot")]
    #[allow(dead_code)]
    FileHistorySnapshot {
        #[serde(rename = "messageId")]
        message_id: String,
        snapshot: serde_json::Value,
        #[serde(rename = "isSnapshotUpdate")]
        is_snapshot_update: bool,
    },
    #[allow(dead_code)]
    System {
        subtype: String,
        level: String,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct UserMessage {
    #[allow(dead_code)]
    role: String,
    content: UserContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UserContent {
    String(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    #[allow(dead_code)]
    role: String,
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        #[allow(dead_code)]
        id: String,
        name: String,
        #[allow(dead_code)]
        input: serde_json::Value,
    },
    #[allow(dead_code)]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
    Thinking {
        thinking: String,
        #[allow(dead_code)]
        signature: String,
    },
}

fn main() {
    let args = Args::parse();

    // Get current working directory
    let current_dir = std::env::current_dir().expect("Failed to get current directory");

    // Convert to Claude projects directory path
    let projects_dir = get_claude_projects_dir(&current_dir);

    if !projects_dir.exists() {
        eprintln!(
            "Claude projects directory not found: {}",
            projects_dir.display()
        );
        eprintln!("Expected directory for: {}", current_dir.display());
        std::process::exit(1);
    }

    // Find all JSONL files
    let jsonl_files = find_jsonl_files(&projects_dir).expect("Failed to find JSONL files");

    if jsonl_files.is_empty() {
        eprintln!("No JSONL files found in {}", projects_dir.display());
        std::process::exit(1);
    }

    // Create fzf input with file previews (filters out empty files)
    let fzf_entries = create_fzf_input(&jsonl_files);

    if fzf_entries.is_empty() {
        eprintln!(
            "No conversations with displayable content found in {}",
            projects_dir.display()
        );
        std::process::exit(1);
    }

    // Run fzf and get selected file
    match run_fzf(&fzf_entries, &jsonl_files) {
        Ok(selected_file) => display_conversation(&selected_file, args.no_tools),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_claude_projects_dir(current_dir: &Path) -> PathBuf {
    let home_dir = std::env::var("HOME").expect("HOME environment variable not set");

    // Convert path to string and replace slashes with dashes
    let path_str = current_dir.to_string_lossy();
    let converted = path_str.replace('/', "-");

    PathBuf::from(home_dir)
        .join(".claude")
        .join("projects")
        .join(converted)
}

fn find_jsonl_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            // Skip agent-generated JSONL files
            if let Some(filename) = path.file_name().and_then(|f| f.to_str())
                && !filename.starts_with("agent-") {
                    files.push(path);
                }
        }
    }

    // Sort by modification time (newest first)
    files.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    files.reverse();

    Ok(files)
}

fn create_fzf_input(files: &[PathBuf]) -> Vec<(usize, String)> {
    files
        .iter()
        .enumerate()
        .filter_map(|(idx, path)| {
            let preview =
                extract_preview(path).unwrap_or_else(|_| "Failed to read file".to_string());
            // Skip files with empty preview (no displayable content)
            if preview.trim().is_empty() {
                return None;
            }
            let timestamp = extract_timestamp(path).unwrap_or_else(|_| "Unknown time".to_string());
            Some((idx, format!("[{}] {} | {}", idx, timestamp, preview)))
        })
        .collect()
}

fn extract_timestamp(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            let timestamp_str = match entry {
                LogEntry::User { timestamp, .. } => Some(timestamp),
                LogEntry::Assistant { timestamp, .. } => Some(timestamp),
                _ => None,
            };

            if let Some(ts) = timestamp_str
                && let Ok(dt) = DateTime::parse_from_rfc3339(&ts) {
                    let local: DateTime<Local> = dt.into();
                    return Ok(local.format("%b %d, %H:%M").to_string());
                }
        }
    }

    Ok("Unknown time".to_string())
}

fn extract_preview(path: &Path) -> std::io::Result<String> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut preview_parts = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            match entry {
                LogEntry::User { message, .. } => {
                    let text = extract_text_from_user(&message);
                    if !text.is_empty() {
                        preview_parts.push(text);
                    }
                }
                LogEntry::Assistant { message, .. } => {
                    let text = extract_text_from_assistant(&message);
                    if !text.is_empty() {
                        preview_parts.push(text);
                    }
                }
                _ => {}
            }
        }

        if preview_parts.len() >= 3 {
            break;
        }
    }

    let preview = preview_parts.join(" ... ");
    // Remove newlines and collapse whitespace to ensure single line
    let preview = preview.replace(['\n', '\r'], " ");
    let preview = preview.split_whitespace().collect::<Vec<_>>().join(" ");

    Ok(preview)
}

fn extract_text_from_user(message: &UserMessage) -> String {
    match &message.content {
        UserContent::String(text) => text.chars().take(100).collect(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text { text } = block {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .take(1)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(100)
            .collect(),
    }
}

fn extract_text_from_assistant(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text { text } = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .take(1)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}

fn run_fzf(
    entries: &[(usize, String)],
    files: &[PathBuf],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut child = Command::new("fzf")
        .args(["--height", "40%", "--reverse", "--border", "--no-multi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        for (_, line) in entries {
            writeln!(stdin, "{}", line)?;
        }
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        return Err("Selection cancelled".into());
    }

    let selection = String::from_utf8_lossy(&output.stdout);
    let selection = selection.trim();

    if selection.is_empty() {
        return Err("No selection made".into());
    }

    // Extract index from [idx] at the start
    if let Some(idx_end) = selection.find(']')
        && idx_end > 1 {
            let idx_str = &selection[1..idx_end];
            if let Ok(idx) = idx_str.parse::<usize>() {
                if idx < files.len() {
                    return Ok(files[idx].clone());
                } else {
                    return Err(format!("Index {} out of range", idx).into());
                }
            }
        }

    Err("Failed to parse selection - expected format: [index] filename | preview".into())
}

fn display_conversation(file_path: &Path, no_tools: bool) {
    let file = File::open(file_path).expect("Failed to open file");
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => display_entry(&entry, no_tools),
            Err(e) => {
                eprintln!("Failed to parse line: {}", e);
                eprintln!("Line content: {}", line);
            }
        }
    }
}

fn display_entry(entry: &LogEntry, no_tools: bool) {
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
            let has_text = message.content.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Text { .. } | ContentBlock::Thinking { .. }
                )
            });
            let tool_uses: Vec<&str> = message
                .content
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { name, .. } = block {
                        Some(name.as_str())
                    } else {
                        None
                    }
                })
                .collect();

            // If there's only tool use blocks and no text, show tool calls (unless no_tools is set)
            if !tool_uses.is_empty() && !has_text && !no_tools {
                for tool_name in tool_uses {
                    println!(
                        "{} <Calling Tool: {}>",
                        "Assistant:".green().bold(),
                        tool_name
                    );
                }
            } else {
                // Show text content and tool calls together
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text } => {
                            println!("{} {}", "Assistant:".green().bold(), text);
                        }
                        ContentBlock::ToolUse { name, .. } => {
                            if !no_tools {
                                println!(
                                    "{} <Calling Tool: {}>",
                                    "Assistant:".green().bold(),
                                    name
                                );
                            }
                        }
                        ContentBlock::Thinking { thinking, .. } => {
                            println!("{} {}", "Thinking:".yellow().bold(), thinking);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
