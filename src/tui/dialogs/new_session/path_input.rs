use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::NewSessionDialog;
use crate::tui::components::PathGhostCompletion;

pub(super) use crate::tui::components::expand_tilde;

/// Compute a path ghost completion for any Input field.
/// Returns None if no completion is available.
pub(super) fn compute_path_ghost(input: &Input) -> Option<PathGhostCompletion> {
    PathGhostCompletion::compute(input)
}

/// Replace home directory prefix with `~` for display.
pub(super) fn collapse_tilde(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path == home_str.as_ref() {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(home_str.as_ref()) {
            if rest.starts_with('/') {
                return format!("~{rest}");
            }
        }
    }
    path.to_string()
}

impl NewSessionDialog {
    pub(super) fn handle_path_shortcuts(&mut self, key: KeyEvent) -> bool {
        if self.focused_field != self.path_field() {
            return false;
        }

        // Right arrow at end of input with ghost: accept ghost text
        if key.code == KeyCode::Right && key.modifiers == KeyModifiers::NONE {
            let cursor = self.path.visual_cursor();
            let char_len = self.path.value().chars().count();
            if cursor >= char_len && self.path_ghost.is_some() {
                self.accept_path_ghost();
                return true;
            }
            return false;
        }

        // End key at end of input with ghost: accept ghost text
        if key.code == KeyCode::End && key.modifiers == KeyModifiers::NONE {
            let cursor = self.path.visual_cursor();
            let char_len = self.path.value().chars().count();
            if cursor >= char_len && self.path_ghost.is_some() {
                self.accept_path_ghost();
                return true;
            }
            return false;
        }

        if matches!(key.code, KeyCode::Home)
            || (key.code == KeyCode::Char('a') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.move_path_cursor_to(0);
            self.error_message = None;
            self.path_invalid_flash_until = None;
            self.recompute_path_ghost();
            return true;
        }

        if (key.code == KeyCode::Left && key.modifiers.contains(KeyModifiers::CONTROL))
            || (key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::ALT))
        {
            self.move_path_cursor_to_previous_segment();
            self.error_message = None;
            self.path_invalid_flash_until = None;
            self.recompute_path_ghost();
            return true;
        }

        false
    }

    fn move_path_cursor_to(&mut self, target_char_idx: usize) {
        let char_len = self.path.value().chars().count();
        let target = target_char_idx.min(char_len);
        let current = self.path.visual_cursor().min(char_len);

        if target < current {
            for _ in 0..(current - target) {
                self.path
                    .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Left,
                        KeyModifiers::NONE,
                    )));
            }
        } else if target > current {
            for _ in 0..(target - current) {
                self.path
                    .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                        KeyCode::Right,
                        KeyModifiers::NONE,
                    )));
            }
        }
    }

    fn move_path_cursor_to_previous_segment(&mut self) {
        let chars: Vec<char> = self.path.value().chars().collect();
        let mut cursor = self.path.visual_cursor().min(chars.len());
        if cursor == 0 {
            return;
        }

        while cursor > 0 && chars[cursor - 1] == '/' {
            cursor -= 1;
        }
        while cursor > 0 && chars[cursor - 1] != '/' {
            cursor -= 1;
        }

        self.move_path_cursor_to(cursor);
    }

    fn set_path_value_with_cursor(&mut self, value: String, cursor_char_idx: usize) {
        self.path = Input::new(value);
        let total_chars = self.path.value().chars().count();
        let target = cursor_char_idx.min(total_chars);
        let left_steps = total_chars.saturating_sub(target);

        for _ in 0..left_steps {
            self.path
                .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                    KeyCode::Left,
                    KeyModifiers::NONE,
                )));
        }
    }

    pub(super) fn recompute_path_ghost(&mut self) {
        self.path_ghost = PathGhostCompletion::compute(&self.path);
    }

    pub(super) fn accept_path_ghost(&mut self) -> bool {
        let ghost = match self.path_ghost.take() {
            Some(g) => g,
            None => return false,
        };
        let Some(new_value) = ghost.accept(&self.path) else {
            return false;
        };
        let new_cursor = new_value.chars().count();
        self.set_path_value_with_cursor(new_value, new_cursor);
        self.error_message = None;
        self.path_invalid_flash_until = None;
        self.recompute_path_ghost();
        true
    }

    pub(super) fn clear_path_ghost(&mut self) {
        self.path_ghost = None;
    }

    pub(super) fn ghost_text(&self) -> Option<&str> {
        self.path_ghost
            .as_ref()
            .map(PathGhostCompletion::ghost_text)
    }

    pub(super) fn is_path_invalid_flash_active(&self) -> bool {
        self.path_invalid_flash_until.is_some()
    }
}
