use crate::claude::{LogEntry, extract_text_from_assistant, extract_text_from_user};
use crate::error::{AppError, Result};
use chrono::{DateTime, Local};
use std::fs::{File, read_dir};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub struct Conversation {
    pub path: PathBuf,
    pub index: usize,
    pub timestamp: DateTime<Local>,
    pub preview: String,
    pub full_text: String,
}

/// Get the Claude projects directory for the current working directory
pub fn get_claude_projects_dir(current_dir: &Path) -> Result<PathBuf> {
    let home_dir = std::env::var("HOME").map_err(|_| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "HOME environment variable not set",
        ))
    })?;

    // Convert path to directory name by replacing slashes with dashes
    // This matches Claude's existing directory naming scheme
    let path_str = current_dir.to_string_lossy();
    let converted = path_str.replace('/', "-");

    Ok(PathBuf::from(home_dir)
        .join(".claude")
        .join("projects")
        .join(converted))
}

/// Find and process all conversation files in one pass
pub fn load_conversations(projects_dir: &Path, show_last: bool) -> Result<Vec<Conversation>> {
    // Find all JSONL files
    let mut file_paths = Vec::new();

    for entry in read_dir(projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("jsonl")
            && let Some(filename) = path.file_name().and_then(|f| f.to_str())
            && !filename.starts_with("agent-")
        {
            file_paths.push(path);
        }
    }

    // Sort by modification time (newest first)
    file_paths.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    file_paths.reverse();

    // Process each file once
    let mut conversations = Vec::new();
    for path in file_paths {
        // Get modification time for display
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

        if let Some(mut conversation) = process_conversation_file(path, show_last, modified)? {
            conversation.index = conversations.len();
            conversations.push(conversation);
        }
    }

    Ok(conversations)
}

/// Process a single conversation file and extract all necessary information
fn process_conversation_file(
    path: PathBuf,
    show_last: bool,
    modified: Option<std::time::SystemTime>,
) -> Result<Option<Conversation>> {
    let file = File::open(&path)?;
    let reader = BufReader::new(file);

    let mut all_parts = Vec::new();
    let mut preview_parts = Vec::new();
    let mut user_messages = Vec::new();
    let mut seen_real_user_message = false;
    let mut skip_next_assistant = false;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            // Extract text content
            match entry {
                LogEntry::User { message, .. } => {
                    let text = extract_text_from_user(&message);
                    if !text.is_empty() {
                        all_parts.push(text.clone());
                        user_messages.push(text.clone());

                        // Check if this is a warmup message (first user message is "Warmup")
                        if !seen_real_user_message && text.trim() == "Warmup" {
                            skip_next_assistant = true;
                        } else {
                            preview_parts.push(text);
                            seen_real_user_message = true;
                        }
                    }
                }
                LogEntry::Assistant { message, .. } => {
                    let text = extract_text_from_assistant(&message);
                    if !text.is_empty() {
                        all_parts.push(text.clone());

                        // Skip this assistant message if it follows a warmup user message
                        if skip_next_assistant {
                            skip_next_assistant = false;
                        } else if seen_real_user_message {
                            // Only add assistant messages to preview after we've seen a real user message
                            preview_parts.push(text);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Check if this is a clear-only conversation or if preview is empty after filtering
    if is_clear_only_conversation(&user_messages)
        || all_parts.is_empty()
        || preview_parts.is_empty()
    {
        return Ok(None);
    }

    // Use file modification time, falling back to current time if unavailable
    let timestamp = modified
        .map(DateTime::<Local>::from)
        .unwrap_or_else(Local::now);

    // Create preview (first or last 3 messages)
    // Skip leading assistant messages by using preview_parts instead of all_parts
    let preview = if show_last {
        preview_parts
            .iter()
            .rev()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ... ")
    } else {
        preview_parts
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ... ")
    };

    // Create full text for searching (all messages)
    let full_text = all_parts.join(" ");

    // Normalize whitespace
    let preview = normalize_whitespace(&preview);
    let full_text = normalize_whitespace(&full_text);

    Ok(Some(Conversation {
        path,
        index: 0,
        timestamp,
        preview,
        full_text,
    }))
}

/// Check if a conversation only contains /clear command messages
fn is_clear_only_conversation(user_messages: &[String]) -> bool {
    if user_messages.is_empty() {
        return false;
    }

    let has_caveat = user_messages.iter().any(|msg| {
        msg.starts_with(
            "Caveat: The messages below were generated by the user while running local commands.",
        )
    });

    let has_command_tags = user_messages
        .iter()
        .any(|msg| msg.contains("<command-name>/clear</command-name>"));

    let has_stdout_tags = user_messages
        .iter()
        .any(|msg| msg.contains("<local-command-stdout>"));

    has_caveat && has_command_tags && has_stdout_tags
}

/// Normalize whitespace in a string
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<&str>>().join(" ")
}
