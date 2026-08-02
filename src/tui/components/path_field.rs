//! A directory-path input: text, ghost completion, and the invalid-path flash.
//!
//! Three dialogs now ask for a working directory, and each of them needs the
//! same segment jumps, ghost acceptance and directory picker. Bundling them
//! here is what keeps the three from drifting apart the way the workspace-repo
//! editor did when it re-implemented a subset of the same behavior.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use std::time::Instant;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::{expand_tilde, DirPicker, PathGhostCompletion};
use crate::tui::styles::Theme;

#[derive(Default)]
pub struct PathField {
    input: Input,
    ghost: Option<PathGhostCompletion>,
    invalid_flash_until: Option<Instant>,
}

impl PathField {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            input: Input::new(value.into()),
            ghost: None,
            invalid_flash_until: None,
        }
    }

    pub fn value(&self) -> &str {
        self.input.value()
    }

    pub fn trimmed(&self) -> String {
        self.input.value().trim().to_string()
    }

    /// The value with a leading `~` expanded, which is the form the filesystem
    /// and tmux both need.
    pub fn resolved(&self) -> String {
        expand_tilde(self.input.value().trim())
    }

    /// Cursor position, in characters from the start of the value.
    pub fn cursor(&self) -> usize {
        self.input.visual_cursor()
    }

    /// Replace the value outright, e.g. from the directory picker.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.input = Input::new(value.into());
        self.invalid_flash_until = None;
        self.recompute_ghost();
    }

    pub fn ghost_text(&self) -> Option<&str> {
        self.ghost.as_ref().map(PathGhostCompletion::ghost_text)
    }

    pub fn recompute_ghost(&mut self) {
        self.ghost = PathGhostCompletion::compute(&self.input);
    }

    pub fn clear_ghost(&mut self) {
        self.ghost = None;
    }

    pub fn is_invalid_flash_active(&self) -> bool {
        self.invalid_flash_until.is_some()
    }

    /// Show the invalid-path colouring until `until`. No caller sets this yet;
    /// the session path field it was moved from never did either.
    #[allow(dead_code)]
    pub fn flash_invalid_until(&mut self, until: Instant) {
        self.invalid_flash_until = Some(until);
    }

    /// Expire the invalid-path flash. Returns true when it just went away and
    /// the caller should redraw.
    pub fn tick(&mut self) -> bool {
        match self.invalid_flash_until {
            Some(until) if Instant::now() >= until => {
                self.invalid_flash_until = None;
                true
            }
            _ => false,
        }
    }

    /// Open `picker` on this field's current value.
    pub fn activate_picker(&self, picker: &mut DirPicker) {
        picker.activate(&self.trimmed());
    }

    /// Handle the path-specific keys: ghost acceptance at the end of the input,
    /// jump to start, and jump to the previous path segment. Returns true when
    /// the key was consumed, in which case the caller should clear whatever
    /// error it is showing.
    pub fn handle_shortcut_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key.code, KeyCode::Right | KeyCode::End) && key.modifiers == KeyModifiers::NONE
        {
            if self.at_end() && self.ghost.is_some() {
                self.accept_ghost();
                return true;
            }
            return false;
        }

        if matches!(key.code, KeyCode::Home)
            || (key.code == KeyCode::Char('a') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.move_cursor_to(0);
            self.invalid_flash_until = None;
            self.recompute_ghost();
            return true;
        }

        if (key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::ALT))
        {
            self.move_cursor_to_previous_segment();
            self.invalid_flash_until = None;
            self.recompute_ghost();
            return true;
        }

        false
    }

    /// Handle an ordinary editing key. Call this only for keys
    /// [`handle_shortcut_key`](Self::handle_shortcut_key) did not consume.
    pub fn handle_text_key(&mut self, key: KeyEvent) {
        self.input.handle_event(&crossterm::event::Event::Key(key));
        self.invalid_flash_until = None;
        self.recompute_ghost();
    }

    pub fn accept_ghost(&mut self) -> bool {
        let Some(ghost) = self.ghost.take() else {
            return false;
        };
        let Some(new_value) = ghost.accept(&self.input) else {
            return false;
        };
        let cursor = new_value.chars().count();
        self.set_value_with_cursor(new_value, cursor);
        self.invalid_flash_until = None;
        self.recompute_ghost();
        true
    }

    /// Render the field as `label value`, with the cursor block, the ghost
    /// completion, and the invalid-path colouring when focused.
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        label: &str,
        is_focused: bool,
        placeholder: Option<&str>,
        theme: &Theme,
    ) {
        let flashing = self.is_invalid_flash_active();
        let color = if flashing {
            theme.error
        } else if is_focused {
            theme.accent
        } else {
            theme.text
        };
        let label_style = if is_focused {
            Style::default().fg(color).underlined()
        } else {
            Style::default().fg(color)
        };
        let value_style = Style::default().fg(color);

        let value = self.value();
        let mut spans = vec![Span::styled(label.to_string(), label_style), Span::raw(" ")];

        if value.is_empty() && !is_focused {
            if let Some(placeholder_text) = placeholder {
                spans.push(Span::styled(placeholder_text.to_string(), value_style));
            }
        } else if is_focused {
            let cursor_pos = self.cursor();
            let cursor_style = if flashing {
                Style::default().fg(theme.background).bg(theme.error)
            } else {
                Style::default().fg(theme.background).bg(theme.accent)
            };

            let before: String = value.chars().take(cursor_pos).collect();
            let cursor_char: String = value
                .chars()
                .nth(cursor_pos)
                .map(|c| c.to_string())
                .unwrap_or_else(|| " ".to_string());
            let after: String = value.chars().skip(cursor_pos + 1).collect();

            if !before.is_empty() {
                spans.push(Span::styled(before, value_style));
            }
            spans.push(Span::styled(cursor_char, cursor_style));
            if !after.is_empty() {
                spans.push(Span::styled(after, value_style));
            }
            if let Some(ghost) = self.ghost_text() {
                spans.push(Span::styled(
                    ghost.to_string(),
                    Style::default().fg(theme.dimmed),
                ));
            }
        } else {
            spans.push(Span::styled(value.to_string(), value_style));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn at_end(&self) -> bool {
        self.cursor() >= self.value().chars().count()
    }

    fn set_value_with_cursor(&mut self, value: String, cursor_char_idx: usize) {
        self.input = Input::new(value);
        let total_chars = self.input.value().chars().count();
        let target = cursor_char_idx.min(total_chars);
        for _ in 0..total_chars.saturating_sub(target) {
            self.send_arrow(KeyCode::Left);
        }
    }

    fn move_cursor_to(&mut self, target_char_idx: usize) {
        let char_len = self.input.value().chars().count();
        let target = target_char_idx.min(char_len);
        let current = self.input.visual_cursor().min(char_len);

        if target < current {
            for _ in 0..(current - target) {
                self.send_arrow(KeyCode::Left);
            }
        } else {
            for _ in 0..(target - current) {
                self.send_arrow(KeyCode::Right);
            }
        }
    }

    fn move_cursor_to_previous_segment(&mut self) {
        let chars: Vec<char> = self.input.value().chars().collect();
        let mut cursor = self.input.visual_cursor().min(chars.len());
        if cursor == 0 {
            return;
        }

        while cursor > 0 && chars[cursor - 1] == '/' {
            cursor -= 1;
        }
        while cursor > 0 && chars[cursor - 1] != '/' {
            cursor -= 1;
        }

        self.move_cursor_to(cursor);
    }

    fn send_arrow(&mut self, code: KeyCode) {
        self.input
            .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                code,
                KeyModifiers::NONE,
            )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn right_at_end_accepts_the_ghost() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

        let mut field = PathField::new(format!("{}/pro", tmp.path().display()));
        field.recompute_ghost();
        assert!(field.handle_shortcut_key(key(KeyCode::Right)));
        assert_eq!(
            field.value(),
            format!("{}/project-alpha/", tmp.path().display())
        );
    }

    #[test]
    fn right_mid_input_is_left_to_the_caller() {
        let mut field = PathField::new("/tmp/alpha".to_string());
        field.handle_shortcut_key(key(KeyCode::Home));
        assert!(!field.handle_shortcut_key(key(KeyCode::Right)));
    }

    #[test]
    fn home_moves_the_cursor_to_the_start() {
        let mut field = PathField::new("/tmp/alpha".to_string());
        assert!(field.handle_shortcut_key(key(KeyCode::Home)));
        assert_eq!(field.cursor(), 0);
    }

    #[test]
    fn ctrl_left_jumps_a_path_segment() {
        let mut field = PathField::new("/tmp/alpha/beta".to_string());
        assert!(field.handle_shortcut_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)));
        assert_eq!(field.cursor(), "/tmp/alpha/".len());
    }

    #[test]
    fn resolved_expands_a_leading_tilde() {
        let home = dirs::home_dir().expect("no home dir");
        let field = PathField::new("~/work ".to_string());
        assert_eq!(field.resolved(), home.join("work").to_string_lossy());
    }

    #[test]
    fn typing_clears_the_invalid_flash() {
        let mut field = PathField::new("/tmp".to_string());
        field.flash_invalid_until(Instant::now() + std::time::Duration::from_secs(60));
        assert!(field.is_invalid_flash_active());
        field.handle_text_key(key(KeyCode::Char('/')));
        assert!(!field.is_invalid_flash_active());
    }
}
