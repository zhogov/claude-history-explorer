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
use std::path::{Path, PathBuf};
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

    // Determine how to load conversations based on mode
    let conversations = if args.global {
        // Mode 1: Global Search (-g) - search all projects at once
        history::load_all_conversations(show_last, args.debug)?
    } else if args.all_projects {
        // Mode 2: Browse Projects (-a) - select project first, then show conversations
        let root = history::get_claude_projects_root()?;

        if !root.exists() {
            return Err(AppError::ProjectsDirNotFound(root.display().to_string()));
        }

        let projects = history::list_projects(&root)?;

        if projects.is_empty() {
            return Err(AppError::NoHistoryFound(root.display().to_string()));
        }

        let selected_project_name = fzf::select_project(&projects)?;
        let projects_dir = root.join(selected_project_name);

        if !projects_dir.exists() {
            return Err(AppError::ProjectsDirNotFound(
                projects_dir.display().to_string(),
            ));
        }

        history::load_conversations(&projects_dir, show_last, args.debug)?
    } else {
        // Mode 3: Current Directory (Default)
        let current_dir = std::env::current_dir().map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Failed to get current directory: {}", e),
            ))
        })?;

        let projects_dir = history::get_claude_projects_dir(&current_dir)?;

        // If --show-dir flag is set, print directory and exit
        if args.show_dir {
            println!("{}", projects_dir.display());
            return Ok(());
        }

        if !projects_dir.exists() {
            return Err(AppError::ProjectsDirNotFound(
                projects_dir.display().to_string(),
            ));
        }

        history::load_conversations(&projects_dir, show_last, args.debug)?
    };

    if conversations.is_empty() {
        return Err(AppError::NoHistoryFound("selected scope".to_string()));
    }

    // Use fzf to select a conversation
    let selected_path = fzf::select_conversation(&conversations, use_relative_time)?;

    if args.show_path {
        println!("{}", selected_path.display());
        return Ok(());
    }

    if args.resume {
        // Find the selected conversation to get its project_path
        let conv = conversations.iter().find(|c| c.path == selected_path);
        if args.debug {
            eprintln!("[DEBUG] Selected path: {}", selected_path.display());
            eprintln!("[DEBUG] Found conversation: {}", conv.is_some());
            if let Some(c) = conv {
                eprintln!("[DEBUG] project_path: {:?}", c.project_path);
                if let Some(p) = &c.project_path {
                    eprintln!("[DEBUG] project_path exists: {}", p.exists());
                }
            }
        }
        let project_path = conv.and_then(|c| c.project_path.as_ref());
        resume_with_claude(&selected_path, project_path)?;
        return Ok(());
    }

    // Display the selected conversation (pass the negative form for no_tools)
    display::display_conversation(&selected_path, !show_tools, show_thinking)?;

    Ok(())
}

fn resume_with_claude(selected_path: &Path, project_path: Option<&PathBuf>) -> Result<()> {
    let conversation_id = selected_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            AppError::ClaudeExecutionError("Conversation filename is not valid Unicode".to_string())
        })?
        .to_owned();

    // Require a valid project directory to resume
    let project_dir = match project_path {
        Some(path) if path.exists() && path.is_dir() => path,
        Some(path) => {
            return Err(AppError::ClaudeExecutionError(format!(
                "Project directory no longer exists: {}",
                path.display()
            )));
        }
        None => {
            return Err(AppError::ClaudeExecutionError(
                "Cannot determine project directory for this conversation".to_string(),
            ));
        }
    };

    let mut command = Command::new("claude");
    command.args(["--resume", &conversation_id]);
    command.current_dir(project_dir);

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
