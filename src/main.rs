mod claude;
mod cli;
mod config;
mod display;
mod error;
mod fzf;
mod history;

use clap::Parser;
use cli::Args;
use error::{AppError, Result};
use std::path::Path;
use std::process::Command;

fn main() {
    if let Err(e) = run() {
        match e {
            AppError::SelectionCancelled => {
                // User cancelled, exit silently
                std::process::exit(0);
            }
            _ => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Helper function to resolve a boolean setting by merging CLI flags and config values.
///
/// Priority: enable_flag > disable_flag > config_value > default_value
fn resolve_bool_setting(
    enable_flag: bool,
    disable_flag: bool,
    config_value: Option<bool>,
    default_value: bool,
) -> bool {
    if enable_flag {
        true
    } else if disable_flag {
        false
    } else {
        config_value.unwrap_or(default_value)
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let config = config::load_config()?;

    // Determine the projects directory
    let projects_dir = if args.all_projects {
        let root = history::get_claude_projects_root()?;

        if !root.exists() {
            return Err(AppError::ProjectsDirNotFound(root.display().to_string()));
        }

        let projects = history::list_projects(&root)?;

        if projects.is_empty() {
            return Err(AppError::NoHistoryFound(root.display().to_string()));
        }

        let selected_project_name = fzf::select_project(&projects)?;
        root.join(selected_project_name)
    } else {
        // Get current working directory
        let current_dir = std::env::current_dir().map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Failed to get current directory: {}", e),
            ))
        })?;

        // Convert to Claude projects directory path
        history::get_claude_projects_dir(&current_dir)?
    };

    // If --show-dir flag is set, print directory and exit
    if args.show_dir {
        println!("{}", projects_dir.display());
        return Ok(());
    }

    // Verify directory exists
    if !projects_dir.exists() {
        return Err(AppError::ProjectsDirNotFound(
            projects_dir.display().to_string(),
        ));
    }

    // Merge CLI arguments with config file settings. CLI takes precedence.
    let display_config = config.display.unwrap_or_default();

    // Use positive names internally for clarity
    let show_tools = resolve_bool_setting(
        args.show_tools,
        args.no_tools,
        display_config.no_tools.map(|b| !b),
        false, // Default: hide tools
    );
    let show_last = resolve_bool_setting(args.last, args.first, display_config.last, false);
    let use_relative_time = resolve_bool_setting(
        args.relative_time,
        args.absolute_time,
        display_config.relative_time,
        false,
    );
    let show_thinking = resolve_bool_setting(
        args.show_thinking,
        args.hide_thinking,
        display_config.show_thinking,
        false,
    );

    // Load all conversations (reads each file once)
    let conversations = history::load_conversations(&projects_dir, show_last, args.debug)?;

    if conversations.is_empty() {
        return Err(AppError::NoHistoryFound(projects_dir.display().to_string()));
    }

    // Use fzf to select a conversation
    let selected_path = fzf::select_conversation(&conversations, use_relative_time)?;

    if args.show_path {
        println!("{}", selected_path.display());
        return Ok(());
    }

    if args.resume {
        resume_with_claude(&selected_path)?;
        return Ok(());
    }

    // Display the selected conversation (pass the negative form for no_tools)
    display::display_conversation(&selected_path, !show_tools, show_thinking)?;

    Ok(())
}

fn resume_with_claude(selected_path: &Path) -> Result<()> {
    let conversation_id = selected_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            AppError::ClaudeExecutionError("Conversation filename is not valid Unicode".to_string())
        })?
        .to_owned();

    let mut command = Command::new("claude");
    command.args(["--resume", &conversation_id]);

    run_claude_command(command)
}

#[cfg(unix)]
fn run_claude_command(mut command: Command) -> Result<()> {
    use std::os::unix::process::CommandExt;

    let err = command.exec();
    Err(AppError::ClaudeExecutionError(err.to_string()))
}

#[cfg(not(unix))]
fn run_claude_command(mut command: Command) -> Result<()> {
    let status = command
        .status()
        .map_err(|e| AppError::ClaudeExecutionError(e.to_string()))?;

    if !status.success() {
        return Err(AppError::ClaudeExecutionError(format!(
            "claude CLI exited with status {}",
            status
        )));
    }

    Ok(())
}
