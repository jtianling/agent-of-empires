use crossterm::event::KeyEvent;
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
        let layout = self.field_layout();
        let field = if self.focused_field == layout.path {
            &mut self.path
        } else if self.focused_field == layout.right_pane_path {
            &mut self.right_pane_path
        } else {
            return false;
        };

        if field.handle_shortcut_key(key) {
            self.error_message = None;
            return true;
        }
        false
    }
}
