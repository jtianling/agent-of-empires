//! Dynamic terminal tab/window title based on TUI state.
//!
//! Uses OSC 0 escape sequences to set the terminal tab title,
//! providing at-a-glance status for users with multiple tabs.

use std::io::{self, Write};

/// Whether any agent session is waiting for user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTitleState {
    /// At least one session's agent is waiting for user input
    AgentWaiting,
    /// No sessions need attention
    Idle,
}

/// Compute the title string for a given state.
pub fn compute_title(state: TabTitleState) -> &'static str {
    match state {
        TabTitleState::AgentWaiting => "\u{270b} AoE",
        TabTitleState::Idle => "\u{25c7} AoE",
    }
}

/// Write an OSC 0 escape sequence to set the terminal tab/window title.
///
/// When running inside tmux, the OSC 0 sequence sets the tmux pane title.
/// The caller (TUI startup in `mod.rs`) is responsible for enabling tmux's
/// `set-titles on` so that pane title changes propagate to the outer terminal.
pub fn set_terminal_title(writer: &mut impl Write, title: &str) -> io::Result<()> {
    write!(writer, "\x1b]0;{title}\x07")?;
    writer.flush()
}

/// Push the current terminal title onto the xterm title stack (CSI 22;2 t).
pub fn push_terminal_title(writer: &mut impl Write) -> io::Result<()> {
    write!(writer, "\x1b[22;2t")?;
    writer.flush()
}

/// Pop and restore the previously pushed title from the xterm title stack (CSI 23;2 t).
pub fn pop_terminal_title(writer: &mut impl Write) -> io::Result<()> {
    write!(writer, "\x1b[23;2t")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_title_all_states() {
        assert_eq!(compute_title(TabTitleState::AgentWaiting), "\u{270b} AoE");
        assert_eq!(compute_title(TabTitleState::Idle), "\u{25c7} AoE");
    }

    #[test]
    fn test_set_terminal_title_writes_osc_sequence() {
        let mut buf = Vec::new();
        set_terminal_title(&mut buf, "test title").unwrap();
        assert_eq!(buf, b"\x1b]0;test title\x07");
    }

    #[test]
    fn test_push_terminal_title_writes_csi_sequence() {
        let mut buf = Vec::new();
        push_terminal_title(&mut buf).unwrap();
        assert_eq!(buf, b"\x1b[22;2t");
    }

    #[test]
    fn test_pop_terminal_title_writes_csi_sequence() {
        let mut buf = Vec::new();
        pop_terminal_title(&mut buf).unwrap();
        assert_eq!(buf, b"\x1b[23;2t");
    }

    #[test]
    fn test_titles_contain_expected_icons() {
        assert!(compute_title(TabTitleState::AgentWaiting).starts_with('\u{270b}'));
        assert!(compute_title(TabTitleState::Idle).starts_with('\u{25c7}'));
    }

    #[test]
    fn test_all_titles_contain_aoe() {
        for state in [TabTitleState::AgentWaiting, TabTitleState::Idle] {
            assert!(
                compute_title(state).contains("AoE"),
                "Title for {:?} should contain 'AoE'",
                state
            );
        }
    }
}
