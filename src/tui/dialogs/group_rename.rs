//! Rename group dialog

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::DialogResult;
use crate::session::validate_group_path;
use crate::tui::components::render_text_field;
use crate::tui::styles::Theme;

pub struct GroupRenameDialog {
    current_path: String,
    new_path: Input,
    error_message: Option<String>,
}

impl GroupRenameDialog {
    pub fn new(current_path: &str) -> Self {
        Self {
            current_path: current_path.to_string(),
            new_path: Input::new(current_path.to_string()),
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
    pub(crate) fn set_path_value(&mut self, path: &str) {
        self.new_path = Input::new(path.to_string());
        self.error_message = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<String> {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                let path = self.new_path.value().trim();
                if path == self.current_path {
                    return DialogResult::Cancel;
                }

                if let Err(err) = validate_group_path(path) {
                    self.error_message = Some(err.to_string());
                    return DialogResult::Continue;
                }

                self.error_message = None;
                DialogResult::Submit(path.to_string())
            }
            _ => {
                self.new_path
                    .handle_event(&crossterm::event::Event::Key(key));
                self.error_message = None;
                DialogResult::Continue
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_area = super::centered_rect(area, 60, 9);

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
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        render_text_field(
            frame,
            chunks[0],
            "Path:",
            &self.new_path,
            true,
            Some("group/path"),
            theme,
        );

        let help = Paragraph::new("Edit the full path to rename or move this group.")
            .style(Style::default().fg(theme.dimmed));
        frame.render_widget(help, chunks[1]);

        let error = self.error_message.as_deref().unwrap_or("");
        let error_style = if self.error_message.is_some() {
            Style::default().fg(theme.error)
        } else {
            Style::default().fg(theme.dimmed)
        };
        frame.render_widget(Paragraph::new(error).style(error_style), chunks[2]);

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
            chunks[3],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_dialog_prefills_current_path() {
        let dialog = GroupRenameDialog::new("work/frontend");
        assert_eq!(dialog.new_path.value(), "work/frontend");
    }

    #[test]
    fn test_dialog_returns_new_path_on_confirm() {
        let mut dialog = GroupRenameDialog::new("work/frontend");
        dialog.new_path = Input::new("work/backend".to_string());

        let result = dialog.handle_key(key(KeyCode::Enter));
        match result {
            DialogResult::Submit(path) => assert_eq!(path, "work/backend"),
            _ => panic!("expected submit"),
        }
    }

    #[test]
    fn test_dialog_rejects_invalid_path() {
        let mut dialog = GroupRenameDialog::new("work/frontend");
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
        let mut dialog = GroupRenameDialog::new("work/frontend");
        let result = dialog.handle_key(key(KeyCode::Enter));
        assert!(matches!(result, DialogResult::Cancel));
    }
}
