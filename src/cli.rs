use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "claude-history")]
#[command(about = "View Claude conversation history with fuzzy search")]
pub struct Args {
    /// Show tool calls in the conversation output
    #[arg(long, short = 't', group = "tools_display")]
    pub show_tools: bool,

    /// Hide tool calls from the conversation output
    #[arg(long, group = "tools_display")]
    pub no_tools: bool,

    /// Show the conversation directory and exit
    #[arg(
        long,
        short = 'd',
        help = "Print the conversation directory path and exit"
    )]
    pub show_dir: bool,

    /// Show the last messages in the fuzzy finder preview
    #[arg(long, short = 'l', group = "preview_content")]
    pub last: bool,

    /// Show the first messages in the fuzzy finder preview
    #[arg(long, group = "preview_content")]
    pub first: bool,

    /// Display relative time (e.g. "10 minutes ago")
    #[arg(long, short = 'r', group = "time_display")]
    pub relative_time: bool,

    /// Display absolute timestamp
    #[arg(long, group = "time_display")]
    pub absolute_time: bool,

    /// Show thinking blocks in the conversation output
    #[arg(long, group = "thinking_display")]
    pub show_thinking: bool,

    /// Hide thinking blocks from the conversation output
    #[arg(long, group = "thinking_display")]
    pub hide_thinking: bool,

    /// Resume the selected conversation in the Claude CLI
    #[arg(
        long,
        short = 'c',
        help = "Resume the selected conversation in Claude Code"
    )]
    pub resume: bool,

    /// Print the selected conversation's file path and exit
    #[arg(long, short = 'p', help = "Print the selected conversation file path")]
    pub show_path: bool,

    /// Show debug output for conversation loading
    #[arg(
        long,
        help = "Print debug information about which conversations were found and filtered"
    )]
    pub debug: bool,

    /// Search conversations from all projects globally
    #[arg(
        long,
        short = 'g',
        help = "Search all conversations from all projects at once"
    )]
    pub global: bool,
}
