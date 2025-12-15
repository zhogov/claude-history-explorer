use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Claude projects directory not found at {0}")]
    ProjectsDirNotFound(String),

    #[error("No conversation history found in {0}")]
    NoHistoryFound(String),

    #[error("Failed to run fzf: {0}")]
    FzfExecutionError(String),

    #[error(
        "fzf version is too old (--freeze-left not supported). Please upgrade to fzf 0.67.0 or later: https://github.com/junegunn/fzf/releases"
    )]
    FzfVersionTooOld,

    #[error("User cancelled selection")]
    SelectionCancelled,

    #[error("Failed to parse fzf selection")]
    FzfSelectionParseError,

    #[error("Index {0} out of range")]
    IndexOutOfRange(usize),

    #[error("Failed to run Claude CLI: {0}")]
    ClaudeExecutionError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
