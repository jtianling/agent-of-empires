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
    pub(super) path: usize,
    pub(super) tool: usize,
    pub(super) right_pane: usize,
    pub(super) right_pane_path: usize,
    pub(super) yolo: usize,
    pub(super) cross_agent_team: usize,
    pub(super) worktree: usize,
    pub(super) new_branch: usize,
    pub(super) sandbox: usize,
    pub(super) group: usize,
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
        let is_terminal = self.is_terminal_selected();
        let has_worktree = !self.worktree_branch.value().is_empty();

        let mut cursor = FieldCursor(0);
        let title = cursor.take();
        let path = cursor.take();
        let tool = cursor.take_if(self.available_tools.len() > 1);
        let right_pane = cursor.take();
        let right_pane_path = cursor.take_if(self.has_right_pane_path_field());
        let yolo = cursor.take_if(self.has_yolo_field());
        let cross_agent_team = cursor.take_if(self.has_cross_agent_team_field());
        let worktree = cursor.take_if(!is_terminal);
        let new_branch = cursor.take_if(!is_terminal && has_worktree);
        let sandbox = cursor.take_if(self.docker_available);
        let group = cursor.take();

        FieldLayout {
            title,
            path,
            tool,
            right_pane,
            right_pane_path,
            yolo,
            cross_agent_team,
            worktree,
            new_branch,
            sandbox,
            group,
            count: cursor.0,
        }
    }

    pub(super) fn path_field(&self) -> usize {
        self.field_layout().path
    }
}
