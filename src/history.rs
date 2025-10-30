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
pub fn load_conversations(
    projects_dir: &Path,
    show_last: bool,
    debug: bool,
) -> Result<Vec<Conversation>> {
    // Find all JSONL files
    let mut file_paths = Vec::new();
    let mut skipped_agent_files = 0;

    for entry in read_dir(projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("jsonl")
            && let Some(filename) = path.file_name().and_then(|f| f.to_str())
        {
            if filename.starts_with("agent-") {
                skipped_agent_files += 1;
                if debug {
                    eprintln!("[DEBUG] Skipping agent file: {}", filename);
                }
            } else {
                file_paths.push(path);
            }
        }
    }

    if debug {
        eprintln!(
            "[DEBUG] Found {} conversation files ({} agent files skipped)",
            file_paths.len(),
            skipped_agent_files
        );
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
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown");

        // Get modification time for display
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

        match process_conversation_file(path.clone(), show_last, modified, debug) {
            Ok(Some(mut conversation)) => {
                if debug {
                    eprintln!("[DEBUG] Loaded {}: {}", filename, conversation.preview);
                }
                conversation.index = conversations.len();
                conversations.push(conversation);
            }
            Ok(None) => {
                // File was filtered - reason already printed in debug output
            }
            Err(e) => {
                if debug {
                    eprintln!("[DEBUG] Error processing {}: {}", filename, e);
                }
            }
        }
    }

    if debug {
        eprintln!(
            "[DEBUG] Total conversations loaded: {}",
            conversations.len()
        );
    }

    Ok(conversations)
}

/// Process a single conversation file and extract all necessary information
fn process_conversation_file(
    path: PathBuf,
    show_last: bool,
    modified: Option<std::time::SystemTime>,
    debug: bool,
) -> Result<Option<Conversation>> {
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unknown");
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
    if is_clear_only_conversation(&user_messages) {
        if debug {
            eprintln!("[DEBUG] Filtered {}: clear-only conversation", filename);
        }
        return Ok(None);
    }

    if all_parts.is_empty() || preview_parts.is_empty() {
        if debug {
            eprintln!(
                "[DEBUG] Filtered {}: empty conversation (all_parts={}, preview_parts={})",
                filename,
                all_parts.len(),
                preview_parts.len()
            );
        }
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

    let mut saw_caveat = false;
    let mut saw_command = false;
    let mut saw_stdout = false;

    for msg in user_messages {
        let trimmed = msg.trim();
        if trimmed.is_empty() {
            continue;
        }

        let is_caveat = trimmed.starts_with(
            "Caveat: The messages below were generated by the user while running local commands.",
        );
        let has_command_tag = trimmed.contains("<command-name>/clear</command-name>");
        let has_stdout_tag = trimmed.contains("<local-command-stdout>");

        if is_caveat {
            saw_caveat = true;
        }
        if has_command_tag {
            saw_command = true;
        }
        if has_stdout_tag {
            saw_stdout = true;
        }

        // Any substantive user message immediately disqualifies this from being clear-only
        if !(is_caveat || has_command_tag || has_stdout_tag) {
            return false;
        }
    }

    saw_caveat && saw_command && saw_stdout
}

/// Normalize whitespace in a string
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<&str>>().join(" ")
}
