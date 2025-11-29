use crate::error::{AppError, Result};
use crate::history::{Conversation, Project};
use chrono::{DateTime, Local};
use chrono_humanize::{Accuracy, HumanTime, Tense};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Run fzf to allow the user to select a conversation
pub fn select_conversation(
    conversations: &[Conversation],
    use_relative_time: bool,
) -> Result<PathBuf> {
    let mut child = Command::new("fzf")
        .args([
            "--height",
            "40%",
            "--reverse",
            "--border",
            "--no-multi",
            "--scheme=default",
            "--delimiter",
            "\t",
            "--with-nth",
            "2",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::FzfExecutionError(e.to_string()))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| AppError::FzfExecutionError("Failed to open stdin".to_string()))?;

        for conv in conversations {
            let timestamp = if use_relative_time {
                format_relative_time(conv.timestamp)
            } else {
                conv.timestamp.format("%b %d, %H:%M").to_string()
            };
            // Include full_text in the display so fzf can search it
            // (fzf doesn't search hidden fields when using --with-nth)
            let display_part = format!(
                "[{}] {} | {} {}",
                conv.index, timestamp, conv.preview, conv.full_text
            );
            // Format: INDEX<tab>DISPLAY_PART
            writeln!(stdin, "{}\t{}", conv.index, display_part)?;
        }
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        return Err(AppError::SelectionCancelled);
    }

    let selection = String::from_utf8_lossy(&output.stdout);
    let selection = selection.trim();

    if selection.is_empty() {
        return Err(AppError::SelectionCancelled);
    }

    // Extract index from the first tab-separated field
    if let Some(idx_str) = selection.split('\t').next()
        && let Ok(idx) = idx_str.parse::<usize>()
    {
        return conversations
            .get(idx)
            .map(|c| c.path.clone())
            .ok_or(AppError::IndexOutOfRange(idx));
    }

    Err(AppError::FzfSelectionParseError)
}

fn format_relative_time(timestamp: DateTime<Local>) -> String {
    let delta = timestamp.signed_duration_since(Local::now());
    HumanTime::from(delta).to_text_en(Accuracy::Rough, Tense::Present)
}

/// Run fzf to allow the user to select a project
pub fn select_project(projects: &[Project]) -> Result<String> {
    let mut child = Command::new("fzf")
        .args([
            "--height",
            "40%",
            "--reverse",
            "--border",
            "--no-multi",
            "--header",
            "Select Project",
            "--delimiter",
            "\t",
            "--with-nth",
            "2",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::FzfExecutionError(e.to_string()))?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| AppError::FzfExecutionError("Failed to open stdin".to_string()))?;

        for project in projects {
            // Format: DIR_NAME<tab>DISPLAY_NAME
            writeln!(stdin, "{}\t{}", project.name, project.display_name)?;
        }
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        return Err(AppError::SelectionCancelled);
    }

    let selection = String::from_utf8_lossy(&output.stdout);
    let selection = selection.trim();

    if selection.is_empty() {
        return Err(AppError::SelectionCancelled);
    }

    // Return the directory name (part before tab)
    selection
        .split('\t')
        .next()
        .map(|s| s.to_string())
        .ok_or(AppError::FzfSelectionParseError)
}
