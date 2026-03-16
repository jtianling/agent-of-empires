//! Terminal title helpers for the AoE TUI lifecycle.

use std::io::{self, Write};

pub fn set_terminal_title(writer: &mut impl Write, title: &str) -> io::Result<()> {
    write!(writer, "\x1b]0;{title}\x07")?;
    writer.flush()
}

pub fn set_tui_title(writer: &mut impl Write, profile: &str) -> io::Result<()> {
    let title = format!("AoE[{profile}]");
    set_terminal_title(writer, &title)
}

pub fn push_terminal_title(writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(b"\x1b[22;2t")?;
    writer.flush()
}

pub fn pop_terminal_title(writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(b"\x1b[23;2t")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_terminal_title_writes_osc_sequence() {
        let mut buf = Vec::new();
        set_terminal_title(&mut buf, "test title").unwrap();
        assert_eq!(buf, b"\x1b]0;test title\x07");
    }

    #[test]
    fn test_set_tui_title_writes_stable_title_with_profile() {
        let mut buf = Vec::new();
        set_tui_title(&mut buf, "my-profile").unwrap();
        assert_eq!(buf, b"\x1b]0;AoE[my-profile]\x07");
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
}
