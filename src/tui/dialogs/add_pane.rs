//! Add-agent-pane dialog.
//!
//! Opens on `%` from the home view for the selected running session. Offers the
//! agent to launch and the directory to launch it in, both defaulting to the
//! session's own. The pane it creates is a peer of the pane beside it, not a
//! copy of it, which is why both are editable.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::DialogResult;
use crate::tui::components::{DirPicker, DirPickerResult, PathField};
use crate::tui::styles::Theme;

/// What the dialog returns on submit.
#[derive(Debug, Clone)]
pub struct AddPaneData {
    pub session_id: String,
    pub tool: String,
    /// The pane's working directory. `None` means the session's own, resolved
    /// when the pane is split.
    pub path: Option<String>,
}

const FIELD_TOOL: usize = 0;
const FIELD_PATH: usize = 1;

pub struct AddPaneDialog {
    session_id: String,
    session_title: String,
    tools: Vec<&'static str>,
    tool_index: usize,
    path: PathField,
    dir_picker: DirPicker,
    focused_field: usize,
    /// A sandboxed session runs its panes through `docker exec -w`, so a host
    /// directory would be accepted, recorded, and then ignored by the process.
    sandboxed: bool,
    error: Option<String>,
}

impl AddPaneDialog {
    pub fn new(
        session_id: &str,
        session_title: &str,
        session_tool: &str,
        tools: Vec<&'static str>,
        sandboxed: bool,
    ) -> Self {
        let tool_index = tools.iter().position(|&t| t == session_tool).unwrap_or(0);
        Self {
            session_id: session_id.to_string(),
            session_title: session_title.to_string(),
            tools,
            tool_index,
            path: PathField::default(),
            dir_picker: DirPicker::new(),
            focused_field: FIELD_TOOL,
            sandboxed,
            error: None,
        }
    }

    fn has_path_field(&self) -> bool {
        !self.sandboxed
    }

    fn field_count(&self) -> usize {
        if self.has_path_field() {
            2
        } else {
            1
        }
    }

    fn selected_tool(&self) -> String {
        self.tools
            .get(self.tool_index)
            .copied()
            .unwrap_or("shell")
            .to_string()
    }

    /// The chosen directory, or `None` for "the session's own".
    ///
    /// Resolved rather than as typed: tmux never expands a leading `~`, so the
    /// literal text would fail the split after passing validation.
    fn selected_path(&self) -> Option<String> {
        if !self.has_path_field() || self.path.trimmed().is_empty() {
            return None;
        }
        Some(self.path.resolved())
    }

    fn is_path_field(&self) -> bool {
        self.has_path_field() && self.focused_field == FIELD_PATH
    }

    /// A directory that does not exist yet fails the split with nothing on
    /// screen to explain it, so it is refused here instead. This dialog offers
    /// no create-directory confirmation: `%` adds a pane to a session that is
    /// already running somewhere, which is not a moment for making directories.
    fn path_problem(&self) -> Option<String> {
        let path = self.selected_path()?;
        let meta = std::fs::metadata(&path);
        match meta {
            Ok(m) if m.is_dir() => None,
            Ok(_) => Some(format!("Not a directory: {path}")),
            Err(_) => Some(format!("Directory not found: {path}")),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<AddPaneData> {
        self.error = None;

        if self.dir_picker.is_active() {
            if let DirPickerResult::Selected(path) = self.dir_picker.handle_key(key) {
                self.path.set_value(path);
            }
            return DialogResult::Continue;
        }

        if self.is_path_field() {
            if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.path.activate_picker(&mut self.dir_picker);
                return DialogResult::Continue;
            }
            if self.path.handle_shortcut_key(key) {
                return DialogResult::Continue;
            }
        }

        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                if let Some(problem) = self.path_problem() {
                    self.error = Some(problem);
                    self.focused_field = FIELD_PATH;
                    return DialogResult::Continue;
                }
                DialogResult::Submit(AddPaneData {
                    session_id: self.session_id.clone(),
                    tool: self.selected_tool(),
                    path: self.selected_path(),
                })
            }
            KeyCode::Tab | KeyCode::Down => {
                self.focused_field = (self.focused_field + 1) % self.field_count();
                DialogResult::Continue
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focused_field = if self.focused_field == 0 {
                    self.field_count() - 1
                } else {
                    self.focused_field - 1
                };
                DialogResult::Continue
            }
            KeyCode::Left if self.focused_field == FIELD_TOOL => {
                let len = self.tools.len().max(1);
                self.tool_index = (self.tool_index + len - 1) % len;
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char(' ') if self.focused_field == FIELD_TOOL => {
                self.tool_index = (self.tool_index + 1) % self.tools.len().max(1);
                DialogResult::Continue
            }
            _ => {
                if self.is_path_field() {
                    self.path.handle_text_key(key);
                }
                DialogResult::Continue
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_width = super::responsive_width(area, 120);
        let dialog_area = super::centered_rect(area, dialog_width, 11);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(" Add Agent Pane ")
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1), // Session
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Agent selector
                Constraint::Length(1), // Path field
                Constraint::Length(1), // Spacer
                Constraint::Min(1),    // Hint
            ])
            .split(inner);

        let session_line = Line::from(vec![
            Span::styled("Session: ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.session_title, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(session_line), chunks[0]);

        let is_tool_focused = self.focused_field == FIELD_TOOL;
        let tool_label_style = if is_tool_focused {
            Style::default().fg(theme.accent).underlined()
        } else {
            Style::default().fg(theme.text)
        };
        let mut tool_spans = vec![Span::styled("Agent:", tool_label_style), Span::raw(" ")];
        for (idx, tool_name) in self.tools.iter().enumerate() {
            let is_selected = idx == self.tool_index;
            let style = if is_selected {
                Style::default().fg(theme.accent).bold()
            } else {
                Style::default().fg(theme.dimmed)
            };
            if idx > 0 {
                tool_spans.push(Span::raw("  "));
            }
            tool_spans.push(Span::styled(if is_selected { "● " } else { "○ " }, style));
            tool_spans.push(Span::styled(*tool_name, style));
        }
        frame.render_widget(Paragraph::new(Line::from(tool_spans)), chunks[2]);

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    err.as_str(),
                    Style::default().fg(theme.error),
                ))),
                chunks[3],
            );
        } else if self.has_path_field() {
            let placeholder = if self.is_path_field() {
                Some("(same as session | Ctrl+P to browse directories)")
            } else {
                Some("(same as session)")
            };
            self.path.render(
                frame,
                chunks[3],
                "Path:",
                self.is_path_field(),
                placeholder,
                theme,
            );
        } else {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Path: (sandboxed session, the container decides)",
                    Style::default().fg(theme.dimmed),
                ))),
                chunks[3],
            );
        }

        let hint = Line::from(vec![
            Span::styled("Tab", Style::default().fg(theme.accent)),
            Span::styled(" next  ", Style::default().fg(theme.dimmed)),
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" agent  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" add  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[5]);

        if self.dir_picker.is_active() {
            self.dir_picker.render(frame, area, theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn dialog() -> AddPaneDialog {
        AddPaneDialog::new(
            "id-1",
            "Carthage",
            "codex",
            vec!["claude", "codex", "shell"],
            false,
        )
    }

    #[test]
    fn defaults_to_the_session_tool_and_directory() {
        let mut dialog = dialog();
        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => {
                assert_eq!(data.tool, "codex");
                assert_eq!(data.path, None, "empty means the session's own directory");
                assert_eq!(data.session_id, "id-1");
            }
            _ => panic!("Expected Submit"),
        }
    }

    #[test]
    fn offers_another_agent_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let mut dialog = dialog();
        dialog.handle_key(key(KeyCode::Right));
        dialog.handle_key(key(KeyCode::Tab));
        for c in path.chars() {
            dialog.handle_key(key(KeyCode::Char(c)));
        }

        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => {
                assert_eq!(data.tool, "shell");
                assert_eq!(data.path.as_deref(), Some(path.as_str()));
            }
            _ => panic!("Expected Submit"),
        }
    }

    /// The directory reaches tmux verbatim, so one that does not exist fails the
    /// split with nothing on screen to explain it.
    #[test]
    fn a_missing_directory_is_refused() {
        let mut dialog = dialog();
        dialog.handle_key(key(KeyCode::Tab));
        for c in "/nonexistent-aoe-add-pane-path".chars() {
            dialog.handle_key(key(KeyCode::Char(c)));
        }

        assert!(matches!(
            dialog.handle_key(key(KeyCode::Enter)),
            DialogResult::Continue
        ));
        assert!(dialog.error.is_some(), "the refusal is surfaced");
    }

    /// A leading `~` is expanded by the shell, never by tmux.
    #[test]
    #[serial_test::serial]
    fn a_tilde_directory_is_submitted_expanded() {
        // Owns `HOME` for the same reason the fork dialog's tilde test does:
        // tests elsewhere point it at a temp dir and never restore it, so the
        // inherited value can name a directory that is already gone, and this
        // dialog refuses a path that does not exist. Restored before the
        // assertion so a failure cannot leave the trap behind.
        let temp = tempfile::tempdir().expect("temp home");
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        let home = dirs::home_dir().expect("home directory");
        let mut dialog = dialog();
        dialog.handle_key(key(KeyCode::Tab));
        dialog.handle_key(key(KeyCode::Char('~')));
        let submitted = dialog.handle_key(key(KeyCode::Enter));

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }

        match submitted {
            DialogResult::Submit(data) => {
                assert_eq!(data.path.as_deref(), Some(home.to_string_lossy().as_ref()));
            }
            _ => panic!("Expected Submit"),
        }
    }

    /// A sandboxed session decides its panes' directory through `docker exec -w`,
    /// so a host path would be recorded and then ignored.
    #[test]
    fn a_sandboxed_session_offers_no_path() {
        let mut dialog = AddPaneDialog::new(
            "id-1",
            "Carthage",
            "codex",
            vec!["claude", "codex", "shell"],
            true,
        );
        assert_eq!(dialog.field_count(), 1, "the path field is not offered");
        dialog.handle_key(key(KeyCode::Tab));
        assert!(!dialog.is_path_field(), "Tab cannot reach it");

        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => assert_eq!(data.path, None),
            _ => panic!("Expected Submit"),
        }
    }

    #[test]
    fn esc_cancels() {
        let mut dialog = dialog();
        assert!(matches!(
            dialog.handle_key(key(KeyCode::Esc)),
            DialogResult::Cancel
        ));
    }

    #[test]
    fn unknown_session_tool_falls_back_to_the_first_offered() {
        let dialog = AddPaneDialog::new(
            "id-1",
            "Carthage",
            "unknown",
            vec!["claude", "codex"],
            false,
        );
        assert_eq!(dialog.selected_tool(), "claude");
    }
}
