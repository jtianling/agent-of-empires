use anyhow::{ensure, Result};

use super::Instance;
use crate::db::{AgentSlot, Store};

impl Instance {
    pub(crate) fn capture_managed_claude_sessions(&self) -> Result<()> {
        self.capture_managed_claude_slots(true)
    }

    pub(super) fn capture_managed_claude_children(&self) -> Result<()> {
        self.capture_managed_claude_slots(false)
    }

    fn capture_managed_claude_slots(&self, include_primary: bool) -> Result<()> {
        if !crate::session::managed_pane::has_managed_state(self) {
            return Ok(());
        }
        let profile = self.managed_profile();
        let storage = crate::session::Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let store = Store::open_with_schema(&profile)?;
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        for row in store.read_slots_for_instance(&self.id)? {
            if row.agent != "claude"
                || row.tmux_pane.is_empty()
                || (!include_primary && row.slot == 0)
            {
                continue;
            }
            let Some((id, native, legacy)) = self.managed_claude_capture_target(&row)? else {
                continue;
            };
            crate::session::managed_pane::require_stored_location(
                &session,
                id,
                &row.tmux_pane,
                legacy,
            )?;
            store.capture_managed_claude_session(&row, native)?;
        }
        Ok(())
    }

    fn managed_claude_capture_target<'a>(
        &'a self,
        row: &AgentSlot,
    ) -> Result<Option<(&'a str, &'a str, bool)>> {
        if row.slot == 0 {
            ensure!(self.tool == "claude", "Managed primary tool changed");
            ensure!(
                self.xats_identity_key.as_deref().unwrap_or("") == row.xats_identity_key,
                "Managed primary identity changed before Claude capture"
            );
            return Ok(self
                .agent_session_id
                .as_deref()
                .map(|native| (self.primary_pane_id(), native, !self.managed_primary_marked)));
        }
        let Some(pane) = self
            .managed_panes
            .iter()
            .find(|pane| pane.slot == Some(row.slot))
        else {
            return Ok(None);
        };
        pane.validate()?;
        ensure!(
            pane.config.tool == "claude"
                && pane.tmux_pane.as_deref() == Some(row.tmux_pane.as_str())
                && pane.identity_key == row.xats_identity_key
                && pane.slot_generation == row.xats_runtime_generation,
            "Managed Claude pane binding changed before capture"
        );
        Ok(pane
            .native_session_id
            .as_deref()
            .map(|native| (pane.id.as_str(), native, false)))
    }
}
