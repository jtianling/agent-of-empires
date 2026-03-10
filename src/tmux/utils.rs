//! tmux utility functions

use std::process::Command;

/// Hook index used for the aoe-specific `client-session-changed` hook.
/// A high index avoids collisions with user-defined hooks.
const AOE_HOOK: &str = "client-session-changed[99]";

/// Sets up a tmux hook that dynamically rebinds `Ctrl+b d` based on the
/// current session:
///
/// - **aoe sessions** (`aoe_*`): switch back to the previous session
///   (falls back to detach if there is no previous session).
/// - **Other sessions**: normal `detach-client` behavior is restored.
///
/// Uses a `client-session-changed` hook so the binding is only active while
/// inside an aoe session and automatically reverts when switching away.
pub fn setup_nested_detach_binding() {
    // Install a hook that rebinds `d` whenever the active session changes.
    Command::new("tmux")
        .args([
            "set-hook",
            "-g",
            AOE_HOOK,
            r#"if-shell "tmux display-message -p '#{session_name}' | grep -q '^aoe_'" "bind-key d run-shell 'tmux switch-client -l 2>/dev/null || tmux detach-client'" "bind-key d detach-client""#,
        ])
        .output()
        .ok();

    // Apply the binding immediately for the current aoe session.
    // The hook only fires on *subsequent* session switches.
    Command::new("tmux")
        .args([
            "bind-key",
            "d",
            "run-shell",
            r#"tmux switch-client -l 2>/dev/null || tmux detach-client"#,
        ])
        .output()
        .ok();
}

/// Removes the hook installed by [`setup_nested_detach_binding`] and restores
/// the default `Ctrl+b d` binding.
pub fn cleanup_nested_detach_binding() {
    Command::new("tmux")
        .args(["set-hook", "-gu", AOE_HOOK])
        .output()
        .ok();
    Command::new("tmux")
        .args(["bind-key", "d", "detach-client"])
        .output()
        .ok();
}

pub fn strip_ansi(content: &str) -> String {
    let mut result = content.to_string();

    while let Some(start) = result.find("\x1b[") {
        let rest = &result[start + 2..];
        let end_offset = rest
            .find(|c: char| c.is_ascii_alphabetic())
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        result = format!("{}{}", &result[..start], &result[start + 2 + end_offset..]);
    }

    while let Some(start) = result.find("\x1b]") {
        if let Some(end) = result[start..].find('\x07') {
            result = format!("{}{}", &result[..start], &result[start + end + 1..]);
        } else {
            break;
        }
    }

    result
}

pub fn sanitize_session_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(20)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_session_name() {
        assert_eq!(sanitize_session_name("my-project"), "my-project");
        assert_eq!(sanitize_session_name("my project"), "my_project");
        assert_eq!(sanitize_session_name("a".repeat(30).as_str()).len(), 20);
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[32mgreen\x1b[0m"), "green");
        assert_eq!(strip_ansi("no codes here"), "no codes here");
        assert_eq!(strip_ansi("\x1b[1;34mbold blue\x1b[0m"), "bold blue");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_strip_ansi_multiple_codes() {
        assert_eq!(
            strip_ansi("\x1b[1m\x1b[32mbold green\x1b[0m normal"),
            "bold green normal"
        );
    }

    #[test]
    fn test_strip_ansi_osc_sequences() {
        assert_eq!(strip_ansi("\x1b]0;Window Title\x07text"), "text");
    }

    #[test]
    fn test_strip_ansi_nested_sequences() {
        assert_eq!(strip_ansi("\x1b[38;5;196mred\x1b[0m"), "red");
    }

    #[test]
    fn test_strip_ansi_with_256_colors() {
        assert_eq!(
            strip_ansi("\x1b[38;2;255;100;50mRGB color\x1b[0m"),
            "RGB color"
        );
    }

    #[test]
    fn test_sanitize_session_name_special_chars() {
        assert_eq!(sanitize_session_name("test/path"), "test_path");
        assert_eq!(sanitize_session_name("test.name"), "test_name");
        assert_eq!(sanitize_session_name("test@name"), "test_name");
        assert_eq!(sanitize_session_name("test:name"), "test_name");
    }

    #[test]
    fn test_sanitize_session_name_preserves_valid_chars() {
        assert_eq!(sanitize_session_name("test-name_123"), "test-name_123");
    }

    #[test]
    fn test_sanitize_session_name_empty() {
        assert_eq!(sanitize_session_name(""), "");
    }

    #[test]
    fn test_sanitize_session_name_unicode() {
        let result = sanitize_session_name("test😀emoji");
        assert!(result.starts_with("test"));
        assert!(result.contains('_'));
        assert!(!result.contains('😀'));
    }
}
