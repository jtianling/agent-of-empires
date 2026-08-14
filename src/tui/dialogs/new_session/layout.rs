//! Field layout for the new session dialog.
//!
//! The dialog shows a conditional sequence of fields, and key handling,
//! rendering and input routing all need the same answer for "which index is
//! this field". Computing it once here is what keeps a newly added conditional
//! field from moving focus onto the wrong row in one of the three.

use super::NewSessionDialog;

/// Index of a field that is not currently displayed. Never equals a real
/// `focused_field`, so comparisons against it are simply false.
pub(super) const ABSENT: usize = usize::MAX;

pub(super) struct FieldLayout {
    pub(super) title: usize,
    pub(super) group: usize,
    pub(super) path: usize,
    pub(super) tool: usize,
    pub(super) yolo: usize,
    pub(super) cross_agent_team: usize,
    pub(super) xats_team: usize,
    pub(super) xats_agent_name: usize,
    pub(super) worktree: usize,
    pub(super) right_pane: usize,
    pub(super) right_pane_path: usize,
    pub(super) right_pane_yolo: usize,
    pub(super) right_pane_cross_agent_team: usize,
    pub(super) right_pane_xats_team: usize,
    pub(super) right_pane_xats_agent_name: usize,
    pub(super) right_pane_worktree: usize,
    /// Number of focusable fields, i.e. the modulus for Tab/BackTab.
    pub(super) count: usize,
}

/// Hands out consecutive indices to the fields that are shown.
struct FieldCursor(usize);

impl FieldCursor {
    fn take(&mut self) -> usize {
        let index = self.0;
        self.0 += 1;
        index
    }

    fn take_if(&mut self, shown: bool) -> usize {
        if shown {
            self.take()
        } else {
            ABSENT
        }
    }
}

impl NewSessionDialog {
    pub(super) fn field_layout(&self) -> FieldLayout {
        let mut cursor = FieldCursor(0);
        let title = cursor.take();
        let group = cursor.take();
        let tool = cursor.take_if(self.available_tools.len() > 1);
        let path = cursor.take();
        let yolo = cursor.take_if(self.pane_has_yolo(super::PaneTarget::Primary));
        let cross_agent_team =
            cursor.take_if(self.pane_has_cross_agent_team(super::PaneTarget::Primary));
        // The declared identity belongs to a pane that talks to xats, so the
        // fields appear with the switch that makes it one. Turning it off does
        // not clear what was typed: the value comes back with the switch.
        let declares_primary = self.pane_declares_xats_identity(super::PaneTarget::Primary);
        let xats_team = cursor.take_if(declares_primary);
        let xats_agent_name = cursor.take_if(declares_primary);
        let worktree = cursor.take();
        let right_pane = cursor.take();
        let right_pane_path = cursor.take_if(self.secondary.is_some());
        let right_pane_yolo = cursor.take_if(self.pane_has_yolo(super::PaneTarget::Secondary));
        let right_pane_cross_agent_team =
            cursor.take_if(self.pane_has_cross_agent_team(super::PaneTarget::Secondary));
        let declares_secondary = self.pane_declares_xats_identity(super::PaneTarget::Secondary);
        let right_pane_xats_team = cursor.take_if(declares_secondary);
        let right_pane_xats_agent_name = cursor.take_if(declares_secondary);
        let right_pane_worktree = cursor.take_if(self.secondary.is_some());

        FieldLayout {
            title,
            group,
            path,
            tool,
            yolo,
            cross_agent_team,
            xats_team,
            xats_agent_name,
            worktree,
            right_pane,
            right_pane_path,
            right_pane_yolo,
            right_pane_cross_agent_team,
            right_pane_xats_team,
            right_pane_xats_agent_name,
            right_pane_worktree,
            count: cursor.0,
        }
    }

    pub(super) fn path_field(&self) -> usize {
        self.field_layout().path
    }
}
