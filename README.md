# claude-history

```
╭──────────────────────────────────────────────────────────────────────────────────────╮
│ ❯ harden                                                                     5/1136  │
│──────────────────────────────────────────────────────────────────────────────────────│
│ ▌ workmux/workmux-merge-cleanup-error                       150 msgs · Jan 14, 21:43 │
│ ▌ ## Description Running `workmux merge` with a bare repository + linked worktrees … │
│ ▌ …ktree Would you like me to apply the hardening (absolute path normalization) to…  │
│──────────────────────────────────────────────────────────────────────────────────────│
│ ▌ raine                                                      68 msgs · Jan 30, 19:36 │
│ ▌ How can I sync my taskwarrior notes between devices? ... There are several approa… │
│ ▌ …` user for the service - Add systemd hardening flags (`NoNewPrivileges`, `Priva…  │
│──────────────────────────────────────────────────────────────────────────────────────│
│ ▌ WalkingMate                                                 2 msgs · Jan 21, 19:25 │
│ ▌ diff --git c/WalkingMate.xcodeproj/project.pbxproj i/WalkingMate.xcodeproj/projec… │
│ ▌ …@@ ENABLE_APP_SANDBOX = YES; ENABLE_HARDENED_RUNTIME = YES; ENABLE_INCOMING_NE…   │
╰──────────────────────────────────────────────────────────────────────────────────────╯
```

`claude-history` is a companion CLI for Claude Code. It lets you search recent
conversations recorded in Claude's local project history with a built-in
terminal UI, then prints the selected transcript in a tidy, readable format.

Run it from the project directory you work on with Claude Code and it will
discover the matching transcript folder automatically.

[Install](#install) · [Usage](#usage) · [Configuration](#configuration) ·
[Changelog](CHANGELOG.md)

## requirements

- Claude Code conversation logs under `~/.claude/projects`

## install

### Homebrew (macOS/Linux)

```sh
brew install raine/claude-history/claude-history
```

### Cargo

```sh
cargo install claude-history
```

## usage

Run the tool from inside the project directory you're interested in:

```sh
$ claude-history
```

This opens a terminal UI listing all conversations, sorted by recency. Type to
search across all transcripts. Each item shows a preview of the conversation and
match context is highlighted when your search matches content not visible in the
preview.

### keyboard navigation

| Key                     | Action                    |
| ----------------------- | ------------------------- |
| `↑` / `↓`               | Move selection            |
| `Ctrl+P` / `Ctrl+N`     | Move selection (vi-style) |
| `Page Up` / `Page Down` | Jump by page              |
| `Home` / `End`          | Jump to first/last        |
| `Enter`                 | Select conversation       |
| `Esc` / `Ctrl+C`        | Quit                      |

### search

Search is case-insensitive substring matching. Results are ranked by a
combination of match frequency and recency, so recent conversations with more
matches appear first.

```
View Claude conversation history

Usage: claude-history [OPTIONS]

Options:
  -t, --show-tools       Show tool calls in the conversation output
      --no-tools         Hide tool calls from the conversation output
  -d, --show-dir         Print the conversation directory path and exit
  -l, --last             Show the last messages in the TUI preview
      --first            Show the first messages in the TUI preview
  -r, --relative-time    Display relative time (e.g. "10 minutes ago")
      --absolute-time    Display absolute timestamp
      --show-thinking    Show thinking blocks in the conversation output
      --hide-thinking    Hide thinking blocks from the conversation output
  -c, --resume           Resume the selected conversation in Claude Code
  -p, --show-path        Print the selected conversation file path
      --debug [<LEVEL>]  Print debug information (filter by level: debug, info, warn, error)
  -g, --global           Search all conversations from all projects at once
  -h, --help             Print help
```

### preview modes

- `claude-history` shows the first messages in the preview
- `claude-history --last` flips the preview to the last messages

### showing tool calls

By default, tool invocations (`<Calling Tool: …>`) are hidden to keep the
conversation focused on the human dialogue. Use `--show-tools` (or `-t`) to
display them when you want to see what tools Claude used.

### showing thinking blocks

Extended thinking models (like Claude Sonnet 4.5) include reasoning steps in
their output. By default, these thinking blocks are hidden to keep conversations
focused. Use `--show-thinking` to display them when you want to see Claude's
reasoning process.

### resuming conversations

If you want to continue a conversation, launch `claude-history` with `--resume`
and it will hand off to `claude --resume <conversation-id>`.

### global search

Use `--global` (or `-g`) to search all conversations from all projects at once:

```sh
$ claude-history --global
```

This displays all conversations from every project in a single view, sorted by
modification time (newest first). Each conversation shows its project path so
you can identify which project it belongs to. Conversations load in the
background so you can start typing immediately.

For [workmux](https://github.com/raine/workmux) users, worktree paths are
displayed in a compact format: `[project/worktree]` instead of just the worktree
folder name.

The `--resume` flag works with global search. It will automatically run Claude
in the correct project directory for the selected conversation.

### integration with other scripts

You can integrate `claude-history` into other tools to pass conversation context
to new Claude Code sessions. This is useful when you want Claude to understand
what you were working on previously.

For example, a commit message generator script could use the conversation
history to write more contextual commit messages:

```bash
# Get conversation history if --context flag is set
conversation_context=""
if [ "$include_history" = true ]; then
    echo "Loading conversation history..."
    conversation_history=$(claude-history 2>/dev/null)
    if [ -n "$conversation_history" ]; then
        conversation_context="

=== START CONVERSATION CONTEXT ===
$conversation_history
=== END CONVERSATION CONTEXT ===

"
    fi
fi

# Pass to Claude CLI with the conversation context
prompt="Write a commit message for these changes.
${conversation_context}
Staged changes:
$staged_diff"

claude -p "$prompt"
```

## configuration

You can set default preferences for display options in
`~/.config/claude-history/config.toml`. Command-line flags will override these
settings.

Create the config file:

```sh
mkdir -p ~/.config/claude-history
cat > ~/.config/claude-history/config.toml << 'EOF'
[display]
# Show tool calls in output (default: false)
no_tools = false

# Show last messages in TUI preview (default: false)
last = false

# Use relative time formatting (default: false)
relative_time = true

# Show thinking blocks (default: false)
show_thinking = false
EOF
```

### available options

- `no_tools` (boolean): When false, shows tool calls; when true, hides them
  (default: false means tools are hidden)
- `last` (boolean): Show last messages instead of first in TUI preview (default:
  false)
- `relative_time` (boolean): Display relative time instead of absolute timestamp
  (default: false)
- `show_thinking` (boolean): Show thinking blocks in conversation output
  (default: false)

### overriding config

Each display option has opposing flags for explicit override:

- `--no-tools` / `--show-tools`
- `--last` / `--first`
- `--relative-time` / `--absolute-time`
- `--hide-thinking` / `--show-thinking`

For example, if your config has `no_tools = false` (showing tools), you can
temporarily hide them with `--no-tools`.

## filtering details

The tool filters out some noisy artifacts before showing conversations, so you
only see transcripts that are likely to matter for your recent work.

- Skips the "Warmup / I'm Claude Code…" exchanges that are sometimes injected
  without user interaction
- Skips conversations that only contain the `/clear` terminal command

## development

The repository includes `just` recipes:

```sh
$ just check
```

This runs `cargo fmt`, `cargo clippy --fix`, and `cargo build` in parallel.

## Related projects

- [workmux](https://github.com/raine/workmux) — Git worktrees + tmux windows for
  parallel AI agent workflows
- [consult-llm-mcp](https://github.com/raine/consult-llm-mcp) — MCP server that
  lets Claude Code consult stronger AI models (o3, Gemini, GPT-5.1 Codex)
- [tmux-file-picker](https://github.com/raine/tmux-file-picker) — Pop up fzf in
  tmux to quickly insert file paths, perfect for AI coding assistants
