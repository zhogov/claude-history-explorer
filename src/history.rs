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
    pub project_name: Option<String>,
    pub project_path: Option<PathBuf>,
    /// The working directory extracted from the JSONL file (the actual cwd)
    pub cwd: Option<PathBuf>,
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

/// Load conversations from ALL projects globally
pub fn load_all_conversations(show_last: bool, debug: bool) -> Result<Vec<Conversation>> {
    let root = get_claude_projects_root()?;
    let projects = list_projects(&root)?;

    if debug {
        eprintln!(
            "[DEBUG] Loading global history from {} projects",
            projects.len()
        );
    }

    // Load conversations from all projects in parallel
    let mut all_conversations: Vec<Conversation> = projects
        .par_iter()
        .flat_map(|project| {
            let project_dir = root.join(&project.name);
            match load_conversations(&project_dir, show_last, debug) {
                Ok(mut convs) => {
                    // Fallback path for old JSONL files without cwd field
                    let fallback_path = decode_project_dir_name_to_path(&project.name);

                    // Inject project info into each conversation
                    for conv in &mut convs {
                        // Prefer the cwd extracted from the JSONL file (accurate), fall back to decoded path
                        let project_path =
                            conv.cwd.clone().unwrap_or_else(|| fallback_path.clone());
                        conv.project_name = Some(format_short_name_from_path(&project_path));
                        conv.project_path = Some(project_path);
                    }
                    convs
                }
                Err(e) => {
                    if debug {
                        eprintln!(
                            "[DEBUG] Failed to load project {}: {}",
                            project.display_name, e
                        );
                    }
                    Vec::new()
                }
            }
        })
        .collect();

    // Global sort by timestamp (newest first)
    all_conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Re-index for fzf selection logic
    for (idx, conv) in all_conversations.iter_mut().enumerate() {
        conv.index = idx;
    }

    if debug {
        eprintln!(
            "[DEBUG] Total global conversations loaded: {}",
            all_conversations.len()
        );
    }

    Ok(all_conversations)
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

/// Format a path into a short display name.
///
/// For worktree paths like `/Users/raine/code/claude-history__worktrees/claude-search`,
/// returns `claude-history/claude-search` to show both the main project and worktree name.
///
/// For regular paths, returns just the folder name.
fn format_short_name_from_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // Check for worktree pattern in the path
    if let Some(wt_pos) = path_str
        .find("__worktrees/")
        .or_else(|| path_str.find("/.worktrees/"))
    {
        let is_hidden = path_str[wt_pos..].starts_with("/.");
        let separator_len = if is_hidden {
            "/.worktrees/".len()
        } else {
            "__worktrees/".len()
        };

        // Get main project (folder before __worktrees)
        let before = &path_str[..wt_pos];
        let main_project = Path::new(before)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Get worktree name (folder after __worktrees/)
        let after = &path_str[wt_pos + separator_len..];
        let worktree = after.split('/').next().unwrap_or("");

        if !main_project.is_empty() && !worktree.is_empty() {
            return format!("{}/{}", main_project, worktree);
        }
    }

    // Not a worktree, just return the folder name
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_str.into_owned())
}

/// Decode a project directory name back to a path (simple heuristic fallback).
///
/// Claude's encoding replaces all non-alphanumeric characters (except `-`) with `-`.
/// This means `/`, `_`, and `.` all become `-`, making the encoding lossy.
///
/// This is only used as a fallback for old JSONL files that don't have the cwd field.
/// The cwd field from JSONL provides the accurate path and should be preferred.
fn decode_project_dir_name_to_path(encoded: &str) -> PathBuf {
    PathBuf::from(decode_with_double_dash_as(encoded, "__"))
}

/// Decode with a specific replacement for double dashes
fn decode_with_double_dash_as(encoded: &str, double_dash_replacement: &str) -> String {
    let mut result = String::with_capacity(encoded.len());
    let mut chars = encoded.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '-' {
            let mut count = 1;
            while chars.peek() == Some(&'-') {
                chars.next();
                count += 1;
            }

            match count {
                1 => result.push('/'),
                2 => result.push_str(double_dash_replacement),
                n => {
                    result.push('/');
                    for _ in 0..((n - 1) / 2) {
                        result.push_str(double_dash_replacement);
                    }
                    if (n - 1) % 2 == 1 {
                        result.push('/');
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Decode a project directory name back to a readable path (for display purposes).
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
    use super::{
        convert_path_to_project_dir_name, decode_project_dir_name, decode_with_double_dash_as,
    };
    use std::path::Path;

    // === Encoding tests ===

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
    fn encodes_worktree_with_double_underscore() {
        let path = Path::new("/Users/raine/code/claude-history__worktrees/claude-search");
        let converted = convert_path_to_project_dir_name(path);
        assert_eq!(
            converted,
            "-Users-raine-code-claude-history--worktrees-claude-search"
        );
    }

    #[test]
    fn encodes_hidden_directory() {
        let path = Path::new("/Users/raine/dotfiles/.config/karabiner");
        let converted = convert_path_to_project_dir_name(path);
        assert_eq!(converted, "-Users-raine-dotfiles--config-karabiner");
    }

    // === Display decode tests (decode_project_dir_name) ===

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

    // === Fallback decode tests (decode_with_double_dash_as) ===

    #[test]
    fn decode_with_double_dash_as_underscore() {
        let encoded = "-Users-raine-code-project--worktrees-feature";
        let decoded = decode_with_double_dash_as(encoded, "__");
        assert_eq!(decoded, "/Users/raine/code/project__worktrees/feature");
    }

    #[test]
    fn decode_with_double_dash_as_hidden_dir() {
        let encoded = "-Users-raine-dotfiles--config-karabiner";
        let decoded = decode_with_double_dash_as(encoded, "/.");
        assert_eq!(decoded, "/Users/raine/dotfiles/.config/karabiner");
    }

    #[test]
    fn decode_preserves_dashes_in_folder_names_in_fallback() {
        // Note: The fallback decode can't distinguish dashes in folder names
        // from path separators - this is expected behavior
        let encoded = "-Users-raine-code-claude-history";
        let decoded = decode_with_double_dash_as(encoded, "__");
        // This incorrectly decodes to /Users/raine/code/claude/history
        // because single dashes are treated as path separators
        assert_eq!(decoded, "/Users/raine/code/claude/history");
    }

    // === Worktree path structure tests ===

    #[test]
    fn worktree_encoded_pattern() {
        // Verify the encoding pattern for worktrees
        let path = Path::new("/Users/raine/code/WalkingMate__worktrees/template-engine");
        let encoded = convert_path_to_project_dir_name(path);
        assert_eq!(
            encoded,
            "-Users-raine-code-WalkingMate--worktrees-template-engine"
        );

        // The --worktrees- pattern should be detectable
        assert!(encoded.contains("--worktrees-"));
    }

    #[test]
    fn extract_worktree_name_from_encoded() {
        let encoded = "-Users-raine-code-WalkingMate--worktrees-template-engine";

        // Find the worktree marker
        let wt_pos = encoded.find("--worktrees-").unwrap();

        // Extract worktree name (everything after --worktrees-)
        let worktree_name = &encoded[wt_pos + "--worktrees-".len()..];
        assert_eq!(worktree_name, "template-engine");
    }

    #[test]
    fn extract_project_name_before_worktrees() {
        let encoded = "-Users-raine-code-WalkingMate--worktrees-template-engine";

        // Find the worktree marker
        let wt_pos = encoded.find("--worktrees-").unwrap();

        // Extract the part before --worktrees
        let before_wt = &encoded[..wt_pos];
        assert_eq!(before_wt, "-Users-raine-code-WalkingMate");

        // When decoded with filesystem check, this should give us WalkingMate as the project name
        // For fallback, it decodes to a path ending in WalkingMate
        let decoded = decode_with_double_dash_as(before_wt, "__");
        assert_eq!(decoded, "/Users/raine/code/WalkingMate");
    }

    // === format_project_short_name tests (worktree display) ===

    #[test]
    fn format_short_name_extracts_worktree_pattern() {
        // Test the worktree pattern detection in decoded paths
        let path = "/Users/raine/code/WalkingMate__worktrees/template-engine";

        // Check for worktree pattern
        assert!(path.contains("__worktrees/"));

        // Extract main project
        let wt_pos = path.find("__worktrees/").unwrap();
        let before = &path[..wt_pos];
        let main_project = Path::new(before)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(main_project, "WalkingMate");

        // Extract worktree name
        let after = &path[wt_pos + "__worktrees/".len()..];
        let worktree = after.split('/').next().unwrap();
        assert_eq!(worktree, "template-engine");

        // Combined display
        let display = format!("{}/{}", main_project, worktree);
        assert_eq!(display, "WalkingMate/template-engine");
    }

    #[test]
    fn format_short_name_hidden_worktrees() {
        // Test .worktrees pattern (hidden worktrees folder)
        let path = "/Users/raine/code/workmux/.worktrees/uncommitted";

        // Check for hidden worktree pattern
        assert!(path.contains("/.worktrees/"));

        let wt_pos = path.find("/.worktrees/").unwrap();
        let before = &path[..wt_pos];
        let main_project = Path::new(before)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap();
        assert_eq!(main_project, "workmux");

        let after = &path[wt_pos + "/.worktrees/".len()..];
        let worktree = after.split('/').next().unwrap();
        assert_eq!(worktree, "uncommitted");
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
    let mut extracted_cwd: Option<PathBuf> = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
            // Extract text content
            match entry {
                LogEntry::User { message, cwd, .. } => {
                    // Extract cwd from the first user message that has it
                    if extracted_cwd.is_none()
                        && let Some(cwd_str) = cwd {
                            extracted_cwd = Some(PathBuf::from(cwd_str));
                        }

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
        project_name: None,
        project_path: None,
        cwd: extracted_cwd,
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
