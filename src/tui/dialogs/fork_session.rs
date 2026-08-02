//! Fork session dialog.
//!
//! Opens on `f` from the home view with the currently selected session as the
//! parent. Lets the user edit the forked title, group, and the right-pane tool
//! (defaulting to `shell` so the shell pane lands in the parent's working
//! directory alongside the native agent fork).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::DialogResult;
use crate::tui::components::{render_text_field, DirPicker, DirPickerResult, PathField};
use crate::tui::styles::Theme;

/// What the dialog returns on submit.
#[derive(Debug, Clone)]
pub struct ForkSessionData {
    pub parent_id: String,
    pub title: String,
    pub group: Option<String>,
    pub right_pane_tool: Option<String>,
    /// Working directory for the right pane. `None` means the forked session's
    /// own, which is the parent's, resolved when the pane is split.
    pub right_pane_path: Option<String>,
}

/// Tools offered for the right pane. Order must match `right_pane_label`.
const RIGHT_PANE_OPTIONS: &[&str] = &["none", "shell", "claude", "codex", "opencode"];

/// Field indices. The path field is shown only when a right pane is selected,
/// so it is the only conditional one and the count follows it.
const FIELD_TITLE: usize = 0;
const FIELD_GROUP: usize = 1;
const FIELD_RIGHT_PANE: usize = 2;
const FIELD_RIGHT_PANE_PATH: usize = 3;

pub struct ForkSessionDialog {
    parent_id: String,
    parent_title: String,
    parent_tool: String,
    title: Input,
    group: Input,
    right_pane_index: usize,
    right_pane_path: PathField,
    dir_picker: DirPicker,
    focused_field: usize,
    parent_sandboxed: bool,
    error: Option<String>,
}

impl ForkSessionDialog {
    pub fn new(
        parent_id: &str,
        parent_title: &str,
        parent_tool: &str,
        parent_group: &str,
        parent_sandboxed: bool,
    ) -> Self {
        let default_title = format!("{}-fork", parent_title);
        Self {
            parent_id: parent_id.to_string(),
            parent_title: parent_title.to_string(),
            parent_tool: parent_tool.to_string(),
            title: Input::new(default_title),
            group: Input::new(parent_group.to_string()),
            // Default to `shell` so the shell pane lands in the same cwd as
            // the parent. Index 1 in RIGHT_PANE_OPTIONS.
            right_pane_index: 1,
            right_pane_path: PathField::default(),
            dir_picker: DirPicker::new(),
            focused_field: FIELD_TITLE,
            parent_sandboxed,
            error: None,
        }
    }

    /// A sandboxed parent forks into a sandboxed session, whose panes take their
    /// directory from `docker exec -w`. Offering a host directory there would
    /// record a value the process never uses.
    fn has_right_pane_path_field(&self) -> bool {
        self.selected_right_pane().is_some() && !self.parent_sandboxed
    }

    fn field_count(&self) -> usize {
        if self.has_right_pane_path_field() {
            4
        } else {
            3
        }
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
    }

    fn focused_input(&mut self) -> Option<&mut Input> {
        match self.focused_field {
            FIELD_TITLE => Some(&mut self.title),
            FIELD_GROUP => Some(&mut self.group),
            _ => None,
        }
    }

    fn is_right_pane_field(&self) -> bool {
        self.focused_field == FIELD_RIGHT_PANE
    }

    fn is_right_pane_path_field(&self) -> bool {
        self.has_right_pane_path_field() && self.focused_field == FIELD_RIGHT_PANE_PATH
    }

    fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % self.field_count();
    }

    fn prev_field(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            self.field_count() - 1
        } else {
            self.focused_field - 1
        };
    }

    /// The right pane's chosen directory, or `None` for "the parent's".
    ///
    /// Resolved rather than as typed: tmux never expands a leading `~`, so the
    /// literal text would fail the split after passing validation.
    fn selected_right_pane_path(&self) -> Option<String> {
        if !self.has_right_pane_path_field() || self.right_pane_path.trimmed().is_empty() {
            return None;
        }
        Some(self.right_pane_path.resolved())
    }

    /// A directory that does not exist yet fails the split with nothing on
    /// screen to explain it, so it is refused here. Forking offers no
    /// create-directory confirmation: the fork itself creates no directories.
    fn right_pane_path_problem(&self) -> Option<String> {
        let path = self.selected_right_pane_path()?;
        match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() => None,
            Ok(_) => Some(format!("Not a directory: {path}")),
            Err(_) => Some(format!("Directory not found: {path}")),
        }
    }

    fn selected_right_pane(&self) -> Option<String> {
        let name = RIGHT_PANE_OPTIONS[self.right_pane_index];
        if name == "none" {
            None
        } else {
            Some(name.to_string())
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<ForkSessionData> {
        self.error = None;

        if self.dir_picker.is_active() {
            if let DirPickerResult::Selected(path) = self.dir_picker.handle_key(key) {
                self.right_pane_path.set_value(path);
            }
            return DialogResult::Continue;
        }

        if self.is_right_pane_path_field() {
            if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                self.right_pane_path.activate_picker(&mut self.dir_picker);
                return DialogResult::Continue;
            }
            if self.right_pane_path.handle_shortcut_key(key) {
                return DialogResult::Continue;
            }
        }

        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                let title_value = self.title.value().trim().to_string();
                if title_value.is_empty() {
                    self.error = Some("Title cannot be empty".to_string());
                    return DialogResult::Continue;
                }
                if let Some(problem) = self.right_pane_path_problem() {
                    self.error = Some(problem);
                    self.focused_field = FIELD_RIGHT_PANE_PATH;
                    return DialogResult::Continue;
                }
                let group_value = self.group.value().trim().to_string();
                let group = if group_value.is_empty() {
                    None
                } else {
                    Some(group_value)
                };
                DialogResult::Submit(ForkSessionData {
                    parent_id: self.parent_id.clone(),
                    title: title_value,
                    group,
                    right_pane_tool: self.selected_right_pane(),
                    right_pane_path: self.selected_right_pane_path(),
                })
            }
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.prev_field();
                } else {
                    self.next_field();
                }
                DialogResult::Continue
            }
            KeyCode::Down => {
                self.next_field();
                DialogResult::Continue
            }
            KeyCode::Up => {
                self.prev_field();
                DialogResult::Continue
            }
            KeyCode::Left if self.is_right_pane_field() => {
                self.right_pane_index = if self.right_pane_index == 0 {
                    RIGHT_PANE_OPTIONS.len() - 1
                } else {
                    self.right_pane_index - 1
                };
                DialogResult::Continue
            }
            KeyCode::Right | KeyCode::Char(' ') if self.is_right_pane_field() => {
                self.right_pane_index = (self.right_pane_index + 1) % RIGHT_PANE_OPTIONS.len();
                DialogResult::Continue
            }
            _ => {
                if self.is_right_pane_path_field() {
                    self.right_pane_path.handle_text_key(key);
                } else if let Some(input) = self.focused_input() {
                    input.handle_event(&crossterm::event::Event::Key(key));
                }
                DialogResult::Continue
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_width = super::responsive_width(area, 120);
        let has_path_field = self.has_right_pane_path_field();
        let dialog_height = if has_path_field { 16 } else { 15 };
        let dialog_area = super::centered_rect(area, dialog_width, dialog_height);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title(" Fork Session ")
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let mut constraints = vec![
            Constraint::Length(1), // Parent
            Constraint::Length(1), // Tool
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Title field
            Constraint::Length(1), // Group field
            Constraint::Length(1), // Right pane selector
        ];
        if has_path_field {
            constraints.push(Constraint::Length(1)); // Right pane path
        }
        constraints.extend([
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Error
            Constraint::Min(1),    // Hint
        ]);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints(constraints)
            .split(inner);

        let parent_line = Line::from(vec![
            Span::styled("Parent: ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.parent_title, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(parent_line), chunks[0]);

        let tool_line = Line::from(vec![
            Span::styled("Tool:   ", Style::default().fg(theme.dimmed)),
            Span::styled(&self.parent_tool, Style::default().fg(theme.text)),
        ]);
        frame.render_widget(Paragraph::new(tool_line), chunks[1]);

        render_text_field(
            frame,
            chunks[3],
            "Title: ",
            &self.title,
            self.focused_field == FIELD_TITLE,
            None,
            theme,
        );
        render_text_field(
            frame,
            chunks[4],
            "Group: ",
            &self.group,
            self.focused_field == FIELD_GROUP,
            Some("(optional)"),
            theme,
        );

        let right_pane_name = RIGHT_PANE_OPTIONS[self.right_pane_index];
        let right_pane_style = if self.is_right_pane_field() {
            Style::default().fg(theme.accent).bold()
        } else {
            Style::default().fg(theme.text)
        };
        let right_pane_line = Line::from(vec![
            Span::styled("Right Pane: ", Style::default().fg(theme.dimmed)),
            Span::styled(format!("◀ {} ▶", right_pane_name), right_pane_style),
        ]);
        frame.render_widget(Paragraph::new(right_pane_line), chunks[5]);

        let mut ci = 6;
        if has_path_field {
            let is_focused = self.is_right_pane_path_field();
            let placeholder = if is_focused {
                Some("(same as parent | Ctrl+P to browse directories)")
            } else {
                Some("(same as parent)")
            };
            self.right_pane_path.render(
                frame,
                chunks[ci],
                "Right Pane Path:",
                is_focused,
                placeholder,
                theme,
            );
            ci += 1;
        }
        ci += 1; // Spacer

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    err.clone(),
                    Style::default().fg(theme.error),
                ))),
                chunks[ci],
            );
        }
        ci += 1;

        let hint = Line::from(vec![
            Span::styled("Tab", Style::default().fg(theme.accent)),
            Span::styled(" next  ", Style::default().fg(theme.dimmed)),
            Span::styled("←/→", Style::default().fg(theme.accent)),
            Span::styled(" cycle  ", Style::default().fg(theme.dimmed)),
            Span::styled("Enter", Style::default().fg(theme.accent)),
            Span::styled(" fork  ", Style::default().fg(theme.dimmed)),
            Span::styled("Esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dimmed)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[ci]);

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

    fn dialog() -> ForkSessionDialog {
        ForkSessionDialog::new("parent-1", "Carthage", "claude", "", false)
    }

    #[test]
    fn an_empty_right_pane_path_means_the_parent_directory() {
        let mut dialog = dialog();

        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => {
                assert_eq!(data.right_pane_tool.as_deref(), Some("shell"));
                assert_eq!(
                    data.right_pane_path, None,
                    "unset follows the forked session, which is the parent's directory"
                );
            }
            _ => panic!("Expected Submit"),
        }
    }

    #[test]
    fn a_named_right_pane_path_is_submitted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let mut dialog = dialog();
        dialog.focused_field = FIELD_RIGHT_PANE_PATH;
        for c in path.chars() {
            dialog.handle_key(key(KeyCode::Char(c)));
        }

        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => {
                assert_eq!(data.right_pane_path.as_deref(), Some(path.as_str()));
            }
            _ => panic!("Expected Submit"),
        }
    }

    /// The directory reaches tmux verbatim, so one that does not exist fails the
    /// split with nothing on screen to explain it.
    #[test]
    fn a_missing_right_pane_path_is_refused() {
        let mut dialog = dialog();
        dialog.focused_field = FIELD_RIGHT_PANE_PATH;
        for c in "/nonexistent-aoe-fork-path".chars() {
            dialog.handle_key(key(KeyCode::Char(c)));
        }

        assert!(matches!(
            dialog.handle_key(key(KeyCode::Enter)),
            DialogResult::Continue
        ));
        assert!(dialog.error.is_some(), "the refusal is surfaced");
    }

    /// A leading `~` is expanded by the shell, never by tmux, so submitting the
    /// typed text would pass validation and then fail the split.
    #[test]
    fn a_tilde_right_pane_path_is_submitted_expanded() {
        let home = dirs::home_dir().expect("home directory");
        let mut dialog = dialog();
        dialog.focused_field = FIELD_RIGHT_PANE_PATH;
        dialog.handle_key(key(KeyCode::Char('~')));

        match dialog.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(data) => {
                assert_eq!(
                    data.right_pane_path.as_deref(),
                    Some(home.to_string_lossy().as_ref())
                );
            }
            _ => panic!("Expected Submit"),
        }
    }

    #[test]
    fn the_path_field_is_hidden_without_a_right_pane() {
        let mut dialog = dialog();
        // Cycle Right Pane back to "none".
        dialog.focused_field = FIELD_RIGHT_PANE;
        dialog.handle_key(key(KeyCode::Left));

        assert_eq!(dialog.selected_right_pane(), None);
        assert_eq!(dialog.field_count(), 3);
    }

    #[test]
    fn tab_reaches_the_path_field_only_when_a_right_pane_is_selected() {
        let mut dialog = dialog();
        for expected in [
            FIELD_GROUP,
            FIELD_RIGHT_PANE,
            FIELD_RIGHT_PANE_PATH,
            FIELD_TITLE,
        ] {
            dialog.handle_key(key(KeyCode::Tab));
            assert_eq!(dialog.focused_field, expected);
        }
    }
}
