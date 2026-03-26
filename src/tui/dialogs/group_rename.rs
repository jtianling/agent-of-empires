//! Rename group dialog

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::DialogResult;
use crate::session::validate_group_path;
use crate::tui::components::{
    expand_tilde, render_text_field, render_text_field_with_ghost, PathGhostCompletion,
};
use crate::tui::styles::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRenameResult {
    pub new_path: String,
    pub directory: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupRenameField {
    Path,
    Directory,
}

impl GroupRenameField {
    fn toggle(self) -> Self {
        match self {
            Self::Path => Self::Directory,
            Self::Directory => Self::Path,
        }
    }
}

pub struct GroupRenameDialog {
    current_path: String,
    new_path: Input,
    directory: Input,
    initial_directory: String,
    dir_ghost: Option<PathGhostCompletion>,
    focused_field: GroupRenameField,
    error_message: Option<String>,
}

impl GroupRenameDialog {
    pub fn new(current_path: &str, current_directory: &str) -> Self {
        Self {
            current_path: current_path.to_string(),
            new_path: Input::new(current_path.to_string()),
            directory: Input::new(current_directory.to_string()),
            initial_directory: current_directory.to_string(),
            dir_ghost: None,
            focused_field: GroupRenameField::Path,
            error_message: None,
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
    }

    #[cfg(test)]
    pub(crate) fn path_value(&self) -> &str {
        self.new_path.value()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_path_value(&mut self, path: &str) {
        self.new_path = Input::new(path.to_string());
        self.error_message = None;
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_directory_value(&mut self, directory: &str) {
        self.directory = Input::new(directory.to_string());
        self.recompute_directory_ghost();
        self.error_message = None;
    }

    fn recompute_directory_ghost(&mut self) {
        self.dir_ghost = PathGhostCompletion::compute(&self.directory);
    }

    fn accept_directory_ghost(&mut self) -> bool {
        let Some(ghost) = self.dir_ghost.take() else {
            return false;
        };
        let Some(new_value) = ghost.accept(&self.directory) else {
            return false;
        };
        self.directory = Input::new(new_value);
        self.error_message = None;
        self.recompute_directory_ghost();
        true
    }

    fn directory_result(&self) -> Option<String> {
        let directory = self.directory.value().trim();
        if directory.is_empty() {
            None
        } else {
            Some(expand_tilde(directory))
        }
    }

    fn handle_path_input(&mut self, key: KeyEvent) -> DialogResult<GroupRenameResult> {
        self.new_path
            .handle_event(&crossterm::event::Event::Key(key));
        self.error_message = None;
        DialogResult::Continue
    }

    fn handle_directory_input(&mut self, key: KeyEvent) -> DialogResult<GroupRenameResult> {
        if matches!(key.code, KeyCode::Right | KeyCode::End) {
            let cursor = self.directory.visual_cursor();
            let char_len = self.directory.value().chars().count();
            if cursor >= char_len && self.dir_ghost.is_some() && self.accept_directory_ghost() {
                return DialogResult::Continue;
            }
        }

        self.directory
            .handle_event(&crossterm::event::Event::Key(key));
        self.error_message = None;
        self.recompute_directory_ghost();
        DialogResult::Continue
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<GroupRenameResult> {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.focused_field = self.focused_field.toggle();
                if self.focused_field == GroupRenameField::Directory {
                    self.recompute_directory_ghost();
                } else {
                    self.dir_ghost = None;
                }
                DialogResult::Continue
            }
            KeyCode::Enter => {
                let path = self.new_path.value().trim();
                let directory = self.directory.value().trim();
                if path == self.current_path && directory == self.initial_directory {
                    return DialogResult::Cancel;
                }

                if let Err(err) = validate_group_path(path) {
                    self.error_message = Some(err.to_string());
                    return DialogResult::Continue;
                }

                self.error_message = None;
                DialogResult::Submit(GroupRenameResult {
                    new_path: path.to_string(),
                    directory: self.directory_result(),
                })
            }
            _ => match self.focused_field {
                GroupRenameField::Path => self.handle_path_input(key),
                GroupRenameField::Directory => self.handle_directory_input(key),
            },
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_area = super::centered_rect(area, 60, 13);

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent))
            .title(" Rename Group ")
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        render_text_field(
            frame,
            chunks[0],
            "Path:",
            &self.new_path,
            self.focused_field == GroupRenameField::Path,
            Some("group/path"),
            theme,
        );

        render_text_field_with_ghost(
            frame,
            chunks[1],
            "Directory:",
            &self.directory,
            self.focused_field == GroupRenameField::Directory,
            Some("/path/to/project"),
            self.dir_ghost.as_ref().map(PathGhostCompletion::ghost_text),
            theme,
        );

        let help = Paragraph::new(
            "Edit the full path to rename or move this group.\nTab/Up/Down switch fields.",
        )
        .style(Style::default().fg(theme.dimmed));
        frame.render_widget(help, chunks[2]);

        let error = self.error_message.as_deref().unwrap_or("");
        let error_style = if self.error_message.is_some() {
            Style::default().fg(theme.error)
        } else {
            Style::default().fg(theme.dimmed)
        };
        frame.render_widget(Paragraph::new(error).style(error_style), chunks[3]);

        let buttons = Line::from(vec![
            Span::styled("[Submit]", Style::default().fg(theme.accent).bold()),
            Span::raw("  "),
            Span::styled("[Cancel]", Style::default().fg(theme.dimmed)),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(theme.hint)),
            Span::raw(" submit  "),
            Span::styled("Esc", Style::default().fg(theme.hint)),
            Span::raw(" cancel"),
        ]);
        frame.render_widget(
            Paragraph::new(buttons).alignment(Alignment::Center),
            chunks[4],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::fs;
    use tempfile::tempdir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_dialog_prefills_current_path() {
        let dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        assert_eq!(dialog.new_path.value(), "work/frontend");
        assert_eq!(dialog.directory.value(), "/tmp/work");
    }

    #[test]
    fn test_dialog_returns_new_path_on_confirm() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.new_path = Input::new("work/backend".to_string());

        let result = dialog.handle_key(key(KeyCode::Enter));
        match result {
            DialogResult::Submit(result) => {
                assert_eq!(result.new_path, "work/backend");
                assert_eq!(result.directory.as_deref(), Some("/tmp/work"));
            }
            _ => panic!("expected submit"),
        }
    }

    #[test]
    fn test_dialog_rejects_invalid_path() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.new_path = Input::new("/invalid".to_string());

        let result = dialog.handle_key(key(KeyCode::Enter));
        assert!(matches!(result, DialogResult::Continue));
        assert_eq!(
            dialog.error_message.as_deref(),
            Some("Group path cannot start or end with '/'")
        );
    }

    #[test]
    fn test_dialog_cancels_when_path_unchanged() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        let result = dialog.handle_key(key(KeyCode::Enter));
        assert!(matches!(result, DialogResult::Cancel));
    }

    #[test]
    fn test_dialog_submits_when_only_directory_changes() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.set_directory_value("/tmp/backend");

        let result = dialog.handle_key(key(KeyCode::Enter));
        match result {
            DialogResult::Submit(result) => {
                assert_eq!(result.new_path, "work/frontend");
                assert_eq!(result.directory.as_deref(), Some("/tmp/backend"));
            }
            _ => panic!("expected submit"),
        }
    }

    #[test]
    fn test_dialog_submits_none_when_directory_is_cleared() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.set_directory_value("");

        let result = dialog.handle_key(key(KeyCode::Enter));
        match result {
            DialogResult::Submit(result) => {
                assert_eq!(result.new_path, "work/frontend");
                assert_eq!(result.directory, None);
            }
            _ => panic!("expected submit"),
        }
    }

    #[test]
    fn test_tab_switches_focus_to_directory() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        assert_eq!(dialog.focused_field, GroupRenameField::Path);

        let result = dialog.handle_key(key(KeyCode::Tab));

        assert!(matches!(result, DialogResult::Continue));
        assert_eq!(dialog.focused_field, GroupRenameField::Directory);
    }

    #[test]
    fn test_up_switches_focus_back_to_path() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.handle_key(key(KeyCode::Tab));

        let result = dialog.handle_key(key(KeyCode::Up));

        assert!(matches!(result, DialogResult::Continue));
        assert_eq!(dialog.focused_field, GroupRenameField::Path);
    }

    #[test]
    fn test_tilde_directory_is_expanded_on_submit() {
        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.set_directory_value("~/projects");

        let result = dialog.handle_key(key(KeyCode::Enter));
        match result {
            DialogResult::Submit(result) => {
                assert!(
                    !result.directory.as_deref().unwrap_or("").starts_with("~/"),
                    "Expected tilde to be expanded, got: {:?}",
                    result.directory
                );
                assert!(
                    result
                        .directory
                        .as_deref()
                        .unwrap_or("")
                        .ends_with("/projects"),
                    "Expected path to end with /projects, got: {:?}",
                    result.directory
                );
            }
            _ => panic!("expected submit"),
        }
    }

    #[test]
    fn test_right_accepts_directory_ghost_completion() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

        let mut dialog = GroupRenameDialog::new("work/frontend", "/tmp/work");
        dialog.handle_key(key(KeyCode::Tab));
        dialog.set_directory_value(&format!("{}/pro", tmp.path().display()));
        assert_eq!(
            dialog
                .dir_ghost
                .as_ref()
                .map(PathGhostCompletion::ghost_text),
            Some("ject-alpha/")
        );

        let result = dialog.handle_key(key(KeyCode::Right));

        assert!(matches!(result, DialogResult::Continue));
        assert_eq!(
            dialog.directory.value(),
            format!("{}/project-alpha/", tmp.path().display())
        );
    }
}
