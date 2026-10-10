//! Info dialog for displaying informational messages

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use unicode_width::UnicodeWidthStr;

use super::DialogResult;
use crate::tui::styles::Theme;

const WIDTH: u16 = 50;
const MIN_HEIGHT: u16 = 9;
/// Borders, the margin around the message, and the button rows.
const CHROME_ROWS: u16 = 6;

pub struct InfoDialog {
    title: String,
    message: String,
}

impl InfoDialog {
    pub fn new(title: &str, message: &str) -> Self {
        Self {
            title: title.to_string(),
            message: message.to_string(),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => DialogResult::Cancel,
            _ => DialogResult::Continue,
        }
    }

    /// Tall enough for the whole message, so a multi-line reason such as a
    /// hook's stderr is not cut off. One spare row absorbs word-wrap slack.
    fn height(&self) -> u16 {
        let text_width = usize::from(WIDTH - 4);
        let rows: usize = self
            .message
            .lines()
            .map(|line| line.width().div_ceil(text_width).max(1))
            .sum();
        u16::try_from(rows + 1)
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME_ROWS)
            .max(MIN_HEIGHT)
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let dialog_area = super::centered_rect(area, WIDTH, self.height());

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(format!(" {} ", self.title))
            .title_style(Style::default().fg(theme.title).bold());

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(inner);

        // Message
        let message = Paragraph::new(&*self.message)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: true });
        frame.render_widget(message, chunks[0]);

        // OK button
        let button = Line::from(vec![Span::styled(
            "[OK]",
            Style::default().fg(theme.accent).bold(),
        )]);

        frame.render_widget(
            Paragraph::new(button).alignment(Alignment::Center),
            chunks[1],
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
    fn test_esc_closes() {
        let mut dialog = InfoDialog::new("Test", "Message");
        let result = dialog.handle_key(key(KeyCode::Esc));
        assert!(matches!(result, DialogResult::Cancel));
    }

    #[test]
    fn test_enter_closes() {
        let mut dialog = InfoDialog::new("Test", "Message");
        let result = dialog.handle_key(key(KeyCode::Enter));
        assert!(matches!(result, DialogResult::Cancel));
    }

    #[test]
    fn test_space_closes() {
        let mut dialog = InfoDialog::new("Test", "Message");
        let result = dialog.handle_key(key(KeyCode::Char(' ')));
        assert!(matches!(result, DialogResult::Cancel));
    }

    #[test]
    fn a_long_message_grows_the_dialog_to_fit() {
        assert_eq!(InfoDialog::new("Test", "Message").height(), MIN_HEIGHT);
        let reason = format!("hook failed:\n\n{}\nboom", "x".repeat(100));
        // 1 + 1 + 3 + 1 message rows, one spare row, then the chrome.
        assert_eq!(InfoDialog::new("Test", &reason).height(), 7 + CHROME_ROWS);
    }

    #[test]
    fn test_other_keys_continue() {
        let mut dialog = InfoDialog::new("Test", "Message");
        let result = dialog.handle_key(key(KeyCode::Char('x')));
        assert!(matches!(result, DialogResult::Continue));
    }
}
