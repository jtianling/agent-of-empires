//! TUI dialog components

use ratatui::layout::Rect;

mod changelog;
mod confirm;
mod custom_instruction;
mod delete_options;
mod fork_session;
mod group_delete_options;
mod group_rename;
mod hook_trust;
mod hooks_install;
mod info;
mod new_session;
mod profile_picker;
mod rename;
mod send_message;
mod welcome;

pub use changelog::ChangelogDialog;
pub use confirm::ConfirmDialog;
pub use custom_instruction::CustomInstructionDialog;
pub use delete_options::{DeleteDialogConfig, DeleteOptions, UnifiedDeleteDialog};
pub use fork_session::{ForkSessionData, ForkSessionDialog};
pub use group_delete_options::{GroupDeleteOptions, GroupDeleteOptionsDialog};
pub use group_rename::{GroupRenameDialog, GroupRenameResult};
pub use hook_trust::{HookTrustAction, HookTrustDialog};
pub use hooks_install::HooksInstallDialog;
pub use info::InfoDialog;
pub use new_session::{NewSessionData, NewSessionDialog};
pub use profile_picker::{ProfileEntry, ProfilePickerAction, ProfilePickerDialog};
pub use rename::{RenameData, RenameDialog, RenameMode};
pub use send_message::SendMessageDialog;
pub use welcome::WelcomeDialog;

pub enum DialogResult<T> {
    Continue,
    Cancel,
    Submit(T),
}

pub fn responsive_width(area: Rect, max: u16) -> u16 {
    area.width.saturating_sub(4).min(max)
}

/// Center a dialog of given size within an area, clamping to fit.
pub fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responsive_width_caps_wide_terminals() {
        let area = Rect::new(0, 0, 160, 40);

        assert_eq!(responsive_width(area, 120), 120);
    }

    #[test]
    fn responsive_width_scales_medium_terminals() {
        let area = Rect::new(0, 0, 100, 40);

        assert_eq!(responsive_width(area, 120), 96);
    }

    #[test]
    fn responsive_width_saturates_tiny_terminals() {
        let area = Rect::new(0, 0, 3, 10);

        assert_eq!(responsive_width(area, 120), 0);
    }
}
