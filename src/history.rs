use crate::claude::{LogEntry, extract_text_from_assistant, extract_text_from_user};
use crate::error::{AppError, Result};
use chrono::{DateTime, Local};
use rayon::prelude::*;
use std::fs::{File, read_dir};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct Conversation {
    pub path: PathBuf,
    pub index: usize,
    pub timestamp: DateTime<Local>,
    pub preview: String,
    pub full_text: String,
}

pub struct Project {
    pub name: String,         // directory name (encoded)
    pub display_name: String, // heuristic decoded path
    pub modified: SystemTime,
}

/// Get the root Claude projects directory (~/.claude/projects)
pub fn get_claude_projects_root() -> Result<PathBuf> {
    let home_dir = std::env::var("HOME").map_err(|_| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "HOME environment variable not set",
        ))
    })?;

    Ok(PathBuf::from(home_dir).join(".claude").join("projects"))
}

/// Get the Claude projects directory for the current working directory
pub fn get_claude_projects_dir(current_dir: &Path) -> Result<PathBuf> {
    let converted = convert_path_to_project_dir_name(current_dir);
    Ok(get_claude_projects_root()?.join(converted))
}

/// List all projects that contain conversation files
pub fn list_projects(root: &Path) -> Result<Vec<Project>> {
    let entries = read_dir(root)?;

    let mut projects: Vec<Project> = entries
        .par_bridge()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();

            if !path.is_dir() {
                return None;
            }

            // Check if project has any non-agent .jsonl files
            let has_conversations = read_dir(&path).ok()?.any(|e| {
                e.ok()
                    .map(|e| {
                        let path = e.path();
                        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        path.extension().map(|s| s == "jsonl").unwrap_or(false)
                            && !name.starts_with("agent-")
                    })
                    .unwrap_or(false)
            });

            if !has_conversations {
                return None;
            }

            let name = path.file_name()?.to_string_lossy().to_string();
            // Heuristic decode: convert encoded directory name back to readable path
            // The encoding replaces non-alphanumeric chars (except -) with -
            // So / becomes -, but _ also becomes -, and __ becomes --
            // We convert single dashes to / but preserve double dashes as _
            let display_name = decode_project_dir_name(&name);
            let modified = entry
                .metadata()
                .ok()?
                .modified()
                .ok()
                .unwrap_or(SystemTime::UNIX_EPOCH);

            Some(Project {
                name,
                display_name,
                modified,
            })
        })
        .collect();

    // Sort by recently modified
    projects.sort_by(|a, b| b.modified.cmp(&a.modified));

    Ok(projects)
}

/// Convert the current working directory into Claude's project directory name.
fn convert_path_to_project_dir_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Decode a project directory name back to a readable path.
///
/// Claude's encoding replaces all non-alphanumeric characters (except `-`) with `-`.
/// This means `/`, `_`, and `.` all become `-`, making the encoding lossy and
/// impossible to reverse perfectly.
///
/// We use a heuristic based on consecutive dash count:
/// - Odd (1, 3, 5...): `/` followed by underscores (e.g. `-` -> `/`, `---` -> `/__`)
/// - Even (2, 4...): All underscores (e.g. `--` -> `__`)
///
/// This prioritizes `__` (common in directory names like git worktrees) over `/_`.
/// Single underscores and dots in the original path will be incorrectly decoded as `/`,
/// but the result is still recognizable enough for project selection in fzf.
fn decode_project_dir_name(encoded: &str) -> String {
    let mut result = String::with_capacity(encoded.len());
    let mut chars = encoded.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '-' {
            // Count consecutive dashes
            let mut count = 1;
            while chars.peek() == Some(&'-') {
                chars.next();
                count += 1;
            }

            if count % 2 == 1 {
                // Odd: first is '/', rest are '_'
                result.push('/');
                for _ in 0..(count - 1) {
                    result.push('_');
                }
            } else {
                // Even: all are '_'
                for _ in 0..count {
                    result.push('_');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::{convert_path_to_project_dir_name, decode_project_dir_name};
    use std::path::Path;

    #[test]
    fn converts_various_separators_and_punctuation() {
        let path = Path::new("/Users/raine/code/workmux/.worktrees/uncommitted");
        let converted = convert_path_to_project_dir_name(path);
        assert_eq!(
            converted,
            "-Users-raine-code-workmux--worktrees-uncommitted"
        );
    }

    #[test]
    fn preserves_alphanumeric_and_existing_dashes() {
        let path = Path::new("/tmp/foo-Bar123");
        let converted = convert_path_to_project_dir_name(path);
        assert_eq!(converted, "-tmp-foo-Bar123");
    }

    #[test]
    fn decodes_consecutive_dashes_to_underscores() {
        // Double dash -> __ (even count = all underscores)
        let encoded = "-Users-raine-code-myproject--worktrees-feature";
        let decoded = decode_project_dir_name(encoded);
        assert_eq!(decoded, "/Users/raine/code/myproject__worktrees/feature");

        // Triple dash -> /__ (odd count = slash + underscores)
        let encoded = "-Users-raine-code-myproject---worktrees-feature";
        let decoded = decode_project_dir_name(encoded);
        assert_eq!(decoded, "/Users/raine/code/myproject/__worktrees/feature");
    }

    #[test]
    fn decodes_single_dashes_to_slashes() {
        let encoded = "-tmp-foo-Bar123";
        let decoded = decode_project_dir_name(encoded);
        assert_eq!(decoded, "/tmp/foo/Bar123");
    }
}

/// Find and process all conversation files in one pass
pub fn load_conversations(
    projects_dir: &Path,
    show_last: bool,
    debug: bool,
) -> Result<Vec<Conversation>> {
    // Find all JSONL files and capture metadata in one pass
    let mut files_with_meta = Vec::new();
    let mut skipped_agent_files = 0;

    for entry in read_dir(projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            if let Some(filename) = path.file_name().and_then(|f| f.to_str())
                && filename.starts_with("agent-")
            {
                skipped_agent_files += 1;
                if debug {
                    eprintln!("[DEBUG] Skipping agent file: {}", filename);
                }
                continue;
            }

            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok());

            files_with_meta.push((path, modified));
        }
    }

    if debug {
        eprintln!(
            "[DEBUG] Found {} conversation files ({} agent files skipped)",
            files_with_meta.len(),
            skipped_agent_files
        );
    }

    // Sort by modification time (newest first)
    files_with_meta.sort_by_key(|(_, modified)| modified.unwrap_or(SystemTime::UNIX_EPOCH));
    files_with_meta.reverse();

    // Process each file (potentially in parallel)
    let mut conversations: Vec<Conversation> = files_with_meta
        .into_par_iter()
        .filter_map(|(path, modified)| {
            let filename = path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("unknown")
                .to_owned();

            match process_conversation_file(path, show_last, modified, debug) {
                Ok(Some(conversation)) => {
                    if debug {
                        eprintln!("[DEBUG] Loaded {}: {}", filename, conversation.preview);
                    }
                    Some(conversation)
                }
                Ok(None) => None,
                Err(e) => {
                    if debug {
                        eprintln!("[DEBUG] Error processing {}: {}", filename, e);
                    }
                    None
                }
            }
        })
        .collect();

    // Ensure deterministic ordering after parallel processing
    conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    for (idx, conv) in conversations.iter_mut().enumerate() {
        conv.index = idx;
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
    modified: Option<SystemTime>,
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
                    if text.is_empty() {
                        continue;
                    }

                    user_messages.push(text.clone());

                    if is_clear_metadata_message(&text) {
                        continue;
                    }

                    all_parts.push(text.clone());

                    // Check if this is a warmup message (first user message is "Warmup")
                    if !seen_real_user_message && text.trim() == "Warmup" {
                        skip_next_assistant = true;
                    } else {
                        preview_parts.push(text);
                        seen_real_user_message = true;
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

/// Detects metadata emitted by the /clear command wrapper messages
fn is_clear_metadata_message(message: &str) -> bool {
    let trimmed = message.trim();

    trimmed.is_empty()
        || trimmed.starts_with(
            "Caveat: The messages below were generated by the user while running local commands.",
        )
        || trimmed.contains("<command-name>/clear</command-name>")
        || trimmed.contains("<command-message>clear</command-message>")
        || trimmed.contains("<local-command-stdout>")
        || trimmed.contains("<command-args>")
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
