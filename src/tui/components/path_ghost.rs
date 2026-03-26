use std::path::PathBuf;
#[cfg(test)]
use std::sync::Mutex;
use tui_input::Input;

use super::text_input::longest_common_prefix;

pub struct PathGhostCompletion {
    input_snapshot: String,
    cursor_snapshot: usize,
    ghost_text: String,
}

fn char_to_byte_idx(value: &str, char_idx: usize) -> usize {
    value
        .char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(value.len())
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn path_completion_base(parent_prefix: &str) -> Option<PathBuf> {
    if parent_prefix.is_empty() {
        return Some(PathBuf::from("."));
    }

    let trimmed = parent_prefix.trim_end_matches('/');
    if trimmed.is_empty() {
        return Some(PathBuf::from("/"));
    }

    if trimmed == "~" {
        return dirs::home_dir();
    }

    if let Some(stripped) = trimmed.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(stripped));
    }

    Some(PathBuf::from(trimmed))
}

impl PathGhostCompletion {
    pub fn compute(input: &Input) -> Option<Self> {
        let value = input.value().to_string();
        let char_len = value.chars().count();
        let cursor_char = input.visual_cursor().min(char_len);

        if cursor_char < char_len {
            return None;
        }

        let cursor_byte = char_to_byte_idx(&value, cursor_char);
        let segment_start = value[..cursor_byte].rfind('/').map_or(0, |idx| idx + 1);
        let parent_prefix = &value[..segment_start];
        let current_segment = &value[segment_start..cursor_byte];

        let base_dir = path_completion_base(parent_prefix)?;

        let include_hidden = current_segment.starts_with('.');
        let mut matches = Vec::new();
        let Ok(entries) = std::fs::read_dir(&base_dir) else {
            return None;
        };

        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            if name.starts_with(current_segment) {
                matches.push(name.to_string());
            }
        }

        if matches.is_empty() {
            return None;
        }
        matches.sort();

        let ghost_text = if matches.len() == 1 {
            let remainder = &matches[0][current_segment.len()..];
            format!("{remainder}/")
        } else {
            let common_prefix = longest_common_prefix(&matches);
            if common_prefix.len() > current_segment.len() {
                common_prefix[current_segment.len()..].to_string()
            } else {
                let remainder = &matches[0][current_segment.len()..];
                format!("{remainder}/")
            }
        };

        if ghost_text.is_empty() {
            return None;
        }

        Some(Self {
            input_snapshot: value,
            cursor_snapshot: cursor_char,
            ghost_text,
        })
    }

    pub fn accept(self, input: &Input) -> Option<String> {
        let value = input.value().to_string();
        let cursor_char = input.visual_cursor().min(value.chars().count());

        if self.input_snapshot != value || self.cursor_snapshot != cursor_char {
            return None;
        }

        let mut new_value = value;
        new_value.push_str(&self.ghost_text);
        Some(new_value)
    }

    pub fn ghost_text(&self) -> &str {
        &self.ghost_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use tempfile::tempdir;
    use tui_input::backend::crossterm::EventHandler;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn move_cursor_left(input: &mut Input) {
        input.handle_event(&crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn compute_returns_single_match_with_trailing_slash() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

        let input = Input::new(format!("{}/pro", tmp.path().display()));
        let ghost = PathGhostCompletion::compute(&input).expect("expected ghost completion");

        assert_eq!(ghost.ghost_text(), "ject-alpha/");
    }

    #[test]
    fn compute_returns_common_prefix_for_multiple_matches() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("client-api")).expect("failed to create directory");
        fs::create_dir(tmp.path().join("client-web")).expect("failed to create directory");

        let input = Input::new(format!("{}/cl", tmp.path().display()));
        let ghost = PathGhostCompletion::compute(&input).expect("expected ghost completion");

        assert_eq!(ghost.ghost_text(), "ient-");
    }

    #[test]
    fn compute_returns_none_when_no_matches_exist() {
        let tmp = tempdir().expect("failed to create temp dir");
        let input = Input::new(format!("{}/zzz_nonexistent", tmp.path().display()));

        assert!(PathGhostCompletion::compute(&input).is_none());
    }

    #[test]
    fn compute_expands_tilde_paths() {
        let _lock = HOME_ENV_LOCK.lock().expect("failed to lock HOME env");
        let tmp = tempdir().expect("failed to create temp dir");
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());

        fs::create_dir(tmp.path().join("projects")).expect("failed to create directory");
        let input = Input::new("~/pro".to_string());
        let ghost = PathGhostCompletion::compute(&input).expect("expected ghost completion");

        assert_eq!(ghost.ghost_text(), "jects/");

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn accept_rejects_stale_ghosts() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("project-alpha")).expect("failed to create directory");

        let input = Input::new(format!("{}/pro", tmp.path().display()));
        let ghost = PathGhostCompletion::compute(&input).expect("expected ghost completion");
        let stale_input = Input::new(format!("{}/proj", tmp.path().display()));

        assert!(ghost.accept(&stale_input).is_none());
    }

    #[test]
    fn compute_returns_none_when_cursor_is_not_at_end() {
        let tmp = tempdir().expect("failed to create temp dir");
        fs::create_dir(tmp.path().join("alpha")).expect("failed to create directory");

        let mut input = Input::new(format!("{}/alp", tmp.path().display()));
        move_cursor_left(&mut input);

        assert!(PathGhostCompletion::compute(&input).is_none());
    }
}
