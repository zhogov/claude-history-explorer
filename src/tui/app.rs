use crate::debug_log;
use crate::error::{AppError, Result};
use crate::history::{Conversation, LoaderMessage};
use crate::tui::search::{self, SearchableConversation};
use crate::tui::ui;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::prelude::*;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Result of running the TUI
pub enum Action {
    Select(PathBuf),
    Delete(PathBuf),
    Quit,
}

/// App mode - normal browsing or confirming an action
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Normal list browsing
    Normal,
    /// Confirming deletion of the selected conversation
    ConfirmDelete,
}

/// Loading state for the TUI
#[derive(Clone, Debug)]
pub enum LoadingState {
    /// Still loading conversations
    Loading { loaded: usize },
    /// All conversations loaded and ready
    Ready,
}

/// App state
pub struct App {
    /// All loaded conversations
    conversations: Vec<Conversation>,
    /// Precomputed search data
    searchable: Vec<SearchableConversation>,
    /// Indices into conversations, sorted by current score
    filtered: Vec<usize>,
    /// Currently selected index into filtered (None if no results)
    selected: Option<usize>,
    /// Current search query
    query: String,
    /// Cursor position in query (character index, not byte)
    cursor_pos: usize,
    /// Whether to use relative time display
    use_relative_time: bool,
    /// Loading state
    loading_state: LoadingState,
    /// Current app mode (normal or confirming deletion)
    mode: Mode,
}

impl App {
    /// Create a new app with all conversations pre-loaded (existing behavior)
    pub fn new(conversations: Vec<Conversation>, use_relative_time: bool) -> Self {
        let searchable = search::precompute_search_text(&conversations);
        let filtered: Vec<usize> = (0..conversations.len()).collect();
        let selected = if filtered.is_empty() { None } else { Some(0) };

        Self {
            conversations,
            searchable,
            filtered,
            selected,
            query: String::new(),
            cursor_pos: 0,
            use_relative_time,
            loading_state: LoadingState::Ready,
            mode: Mode::Normal,
        }
    }

    /// Create a new app in loading state
    pub fn new_loading(use_relative_time: bool) -> Self {
        Self {
            conversations: Vec::new(),
            searchable: Vec::new(),
            filtered: Vec::new(),
            selected: None,
            query: String::new(),
            cursor_pos: 0,
            use_relative_time,
            loading_state: LoadingState::Loading { loaded: 0 },
            mode: Mode::Normal,
        }
    }

    /// Append a batch of conversations during loading
    /// Note: Does NOT precompute search text - that's deferred to finish_loading
    pub fn append_conversations(&mut self, new_convs: Vec<Conversation>) {
        let start_idx = self.conversations.len();
        self.conversations.extend(new_convs);
        let end_idx = self.conversations.len();

        // Update filtered so items appear in the list during loading
        // (Items shown in arrival order initially, will be re-sorted in finish_loading)
        self.filtered.extend(start_idx..end_idx);

        // Select first item if nothing selected yet
        if self.selected.is_none() && !self.filtered.is_empty() {
            self.selected = Some(0);
        }

        // Update loading count
        self.loading_state = LoadingState::Loading {
            loaded: self.conversations.len(),
        };
    }

    /// Mark loading as complete: sort, precompute search, and transition to Ready
    pub fn finish_loading(&mut self) {
        // Sort all conversations by timestamp (newest first)
        self.conversations
            .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Reindex after sorting
        for (idx, conv) in self.conversations.iter_mut().enumerate() {
            conv.index = idx;
        }

        // Now precompute search text (only once, at the end)
        self.searchable = search::precompute_search_text(&self.conversations);

        self.loading_state = LoadingState::Ready;

        // Apply any query that was typed during loading
        if self.query.is_empty() {
            // Reset filtered to all indices
            self.filtered = (0..self.conversations.len()).collect();
            self.selected = if self.filtered.is_empty() {
                None
            } else {
                Some(0)
            };
        } else {
            // User typed during loading, apply the filter now
            self.update_filter();
        }
    }

    /// Consume the app and return its conversations
    pub fn into_conversations(self) -> Vec<Conversation> {
        self.conversations
    }

    pub fn loading_state(&self) -> &LoadingState {
        &self.loading_state
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.loading_state, LoadingState::Loading { .. })
    }

    /// Update filtered results based on current query
    fn update_filter(&mut self) {
        let now = Local::now();
        self.filtered = search::search(&self.conversations, &self.searchable, &self.query, now);
        self.selected = if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Move selection up
    fn select_prev(&mut self) {
        if let Some(selected) = self.selected
            && selected > 0
        {
            self.selected = Some(selected - 1);
        }
    }

    /// Move selection down
    fn select_next(&mut self) {
        if let Some(selected) = self.selected
            && selected + 1 < self.filtered.len()
        {
            self.selected = Some(selected + 1);
        }
    }

    /// Move selection to first item
    fn select_first(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = Some(0);
        }
    }

    /// Move selection to last item
    fn select_last(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = Some(self.filtered.len() - 1);
        }
    }

    /// Move selection up by a page
    fn select_page_up(&mut self) {
        if let Some(selected) = self.selected {
            self.selected = Some(selected.saturating_sub(10));
        }
    }

    /// Move selection down by a page
    fn select_page_down(&mut self) {
        if let Some(selected) = self.selected {
            let new_selected = (selected + 10).min(self.filtered.len().saturating_sub(1));
            self.selected = Some(new_selected);
        }
    }

    /// Get the currently selected conversation path
    fn get_selected_path(&self) -> Option<PathBuf> {
        self.selected
            .and_then(|sel| self.filtered.get(sel))
            .map(|&idx| self.conversations[idx].path.clone())
    }

    // Getters for UI access
    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    pub fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn use_relative_time(&self) -> bool {
        self.use_relative_time
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn cursor_pos(&self) -> usize {
        self.cursor_pos
    }

    /// Move cursor left by one character
    fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Move cursor right by one character
    fn cursor_right(&mut self) {
        let len = self.query.chars().count();
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
    }

    /// Remove the currently selected conversation from the UI list.
    /// This should only be called after the file has been successfully deleted from disk.
    /// Handles index management for conversations, searchable, and filtered vectors.
    pub fn remove_selected_from_list(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(&conv_idx) = self.filtered.get(selected) else {
            return;
        };

        // Remove from conversations
        self.conversations.remove(conv_idx);

        // Remove from searchable if it's populated (empty during loading)
        if conv_idx < self.searchable.len() {
            self.searchable.remove(conv_idx);
        }

        // Update filtered: remove the deleted index and decrement all indices > conv_idx
        self.filtered.retain(|&idx| idx != conv_idx);
        for idx in &mut self.filtered {
            if *idx > conv_idx {
                *idx -= 1;
            }
        }

        // Update selection: stay at same position if possible, or move to last item
        if self.filtered.is_empty() {
            self.selected = None;
        } else if selected >= self.filtered.len() {
            self.selected = Some(self.filtered.len() - 1);
        }
        // else: selected stays the same (now pointing to next item)
    }

    /// Handle a key event during confirmation mode
    fn handle_confirm_key(&mut self, code: KeyCode) -> Option<Action> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.mode = Mode::Normal;
                self.get_selected_path().map(Action::Delete)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.mode = Mode::Normal;
                None
            }
            _ => None,
        }
    }

    /// Handle a key event, returns Some(Action) if the app should exit
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        // Handle confirmation mode first
        if self.mode == Mode::ConfirmDelete {
            return self.handle_confirm_key(code);
        }

        // During loading, allow navigation and typing but not Enter selection
        if self.is_loading() {
            return match code {
                KeyCode::Esc => Some(Action::Quit),
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Action::Quit)
                }
                KeyCode::Left => {
                    self.cursor_left();
                    None
                }
                KeyCode::Right => {
                    self.cursor_right();
                    None
                }
                KeyCode::Up => {
                    self.select_prev();
                    None
                }
                KeyCode::Down => {
                    self.select_next();
                    None
                }
                KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.select_next();
                    None
                }
                KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                    self.select_prev();
                    None
                }
                KeyCode::PageUp => {
                    self.select_page_up();
                    None
                }
                KeyCode::PageDown => {
                    self.select_page_down();
                    None
                }
                // Allow typing during loading - query is buffered for when loading finishes
                KeyCode::Char(c) => {
                    // Insert at cursor position
                    let byte_pos = self
                        .query
                        .char_indices()
                        .nth(self.cursor_pos)
                        .map(|(i, _)| i)
                        .unwrap_or(self.query.len());
                    self.query.insert(byte_pos, c);
                    self.cursor_pos += 1;
                    None
                }
                KeyCode::Backspace => {
                    if self.cursor_pos > 0
                        && let Some((byte_pos, _)) =
                            self.query.char_indices().nth(self.cursor_pos - 1)
                    {
                        self.query.remove(byte_pos);
                        self.cursor_pos -= 1;
                    }
                    None
                }
                KeyCode::Delete => {
                    let len = self.query.chars().count();
                    if self.cursor_pos < len
                        && let Some((byte_pos, _)) = self.query.char_indices().nth(self.cursor_pos)
                    {
                        self.query.remove(byte_pos);
                    }
                    None
                }
                _ => None,
            };
        }

        // Normal handling when ready
        match code {
            KeyCode::Esc => Some(Action::Quit),
            KeyCode::Enter => self.get_selected_path().map(Action::Select),
            KeyCode::Left => {
                self.cursor_left();
                None
            }
            KeyCode::Right => {
                self.cursor_right();
                None
            }
            KeyCode::Up => {
                self.select_prev();
                None
            }
            KeyCode::Down => {
                self.select_next();
                None
            }
            KeyCode::Home => {
                self.select_first();
                None
            }
            KeyCode::End => {
                self.select_last();
                None
            }
            KeyCode::PageUp => {
                self.select_page_up();
                None
            }
            KeyCode::PageDown => {
                self.select_page_down();
                None
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Quit),
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_next();
                None
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.select_prev();
                None
            }
            KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
                if self.get_selected_path().is_some() {
                    self.mode = Mode::ConfirmDelete;
                }
                None
            }
            KeyCode::Char(c) => {
                // Insert at cursor position
                let byte_pos = self
                    .query
                    .char_indices()
                    .nth(self.cursor_pos)
                    .map(|(i, _)| i)
                    .unwrap_or(self.query.len());
                self.query.insert(byte_pos, c);
                self.cursor_pos += 1;
                self.update_filter();
                None
            }
            KeyCode::Backspace => {
                if self.cursor_pos > 0
                    && let Some((byte_pos, _)) = self.query.char_indices().nth(self.cursor_pos - 1)
                {
                    self.query.remove(byte_pos);
                    self.cursor_pos -= 1;
                }
                self.update_filter();
                None
            }
            KeyCode::Delete => {
                let len = self.query.chars().count();
                if self.cursor_pos < len
                    && let Some((byte_pos, _)) = self.query.char_indices().nth(self.cursor_pos)
                {
                    self.query.remove(byte_pos);
                }
                self.update_filter();
                None
            }
            _ => None,
        }
    }
}

/// RAII guard to ensure terminal is restored on exit
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode().map_err(|e| AppError::Io(io::Error::other(e)))?;

        let mut stdout = io::stdout();
        if let Err(e) = crossterm::execute!(stdout, EnterAlternateScreen) {
            let _ = terminal::disable_raw_mode();
            return Err(AppError::Io(io::Error::other(e)));
        }

        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(t) => t,
            Err(e) => {
                let _ = terminal::disable_raw_mode();
                let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
                return Err(AppError::Io(io::Error::other(e)));
            }
        };

        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

/// Run the TUI and return the selected conversation path or None if cancelled
pub fn run(conversations: Vec<Conversation>, use_relative_time: bool) -> Result<Action> {
    // Set up panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut guard = TerminalGuard::new()?;
    let mut app = App::new(conversations, use_relative_time);

    loop {
        guard.terminal.draw(|frame| ui::render(frame, &app))?;

        if let Event::Key(key) = event::read().map_err(|e| AppError::Io(io::Error::other(e)))? {
            // Only handle key press events (not release)
            if key.kind == KeyEventKind::Press
                && let Some(action) = app.handle_key(key.code, key.modifiers)
            {
                match action {
                    Action::Delete(ref path) => {
                        // Delete the file from disk
                        match std::fs::remove_file(path) {
                            Ok(()) => {
                                // Only remove from list if file deletion succeeded
                                app.remove_selected_from_list();
                            }
                            Err(e) => {
                                let _ = debug_log::log_debug(&format!(
                                    "Failed to delete {}: {}",
                                    path.display(),
                                    e
                                ));
                                // Keep item in list since file still exists
                            }
                        }
                        // Continue the loop (don't exit TUI)
                    }
                    Action::Select(ref path) => {
                        let _ = debug_log::log_selected_path(path);
                        return Ok(action);
                    }
                    Action::Quit => return Ok(action),
                }
            }
        }
    }
}

/// Run the TUI with background loading
/// Returns the action and the final list of conversations
pub fn run_with_loader(
    rx: Receiver<LoaderMessage>,
    use_relative_time: bool,
) -> Result<(Action, Vec<Conversation>)> {
    // Set up panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut guard = TerminalGuard::new()?;
    let mut app = App::new_loading(use_relative_time);

    loop {
        // Process all pending loader messages (non-blocking)
        loop {
            match rx.try_recv() {
                Ok(LoaderMessage::Fatal(err)) => {
                    // Fatal error - restore terminal and return error
                    drop(guard);
                    return Err(err);
                }
                Ok(LoaderMessage::ProjectError) => {
                    // Logged by loader, continue
                }
                Ok(LoaderMessage::Batch(convs)) => {
                    app.append_conversations(convs);
                }
                Ok(LoaderMessage::Done) => {
                    app.finish_loading();
                    // Check for empty conversations
                    if app.conversations().is_empty() {
                        drop(guard);
                        return Err(AppError::NoHistoryFound("selected scope".to_string()));
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Loader finished unexpectedly
                    if app.is_loading() {
                        app.finish_loading();
                        if app.conversations().is_empty() {
                            drop(guard);
                            return Err(AppError::NoHistoryFound("selected scope".to_string()));
                        }
                    }
                    break;
                }
            }
        }

        // Render current state
        guard.terminal.draw(|frame| ui::render(frame, &app))?;

        // Poll for keyboard input with timeout (allows us to check loader messages)
        if event::poll(Duration::from_millis(50)).map_err(|e| AppError::Io(io::Error::other(e)))?
            && let Event::Key(key) = event::read().map_err(|e| AppError::Io(io::Error::other(e)))?
            && key.kind == KeyEventKind::Press
            && let Some(action) = app.handle_key(key.code, key.modifiers)
        {
            match action {
                Action::Delete(ref path) => {
                    // Delete the file from disk
                    match std::fs::remove_file(path) {
                        Ok(()) => {
                            // Only remove from list if file deletion succeeded
                            app.remove_selected_from_list();
                        }
                        Err(e) => {
                            let _ = debug_log::log_debug(&format!(
                                "Failed to delete {}: {}",
                                path.display(),
                                e
                            ));
                            // Keep item in list since file still exists
                        }
                    }
                    // Continue the loop (don't exit TUI)
                }
                _ => return Ok((action, app.into_conversations())),
            }
        }
    }
}
