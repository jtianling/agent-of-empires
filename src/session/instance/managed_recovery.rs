use anyhow::{ensure, Context, Result};

use super::Instance;
use crate::db::{AgentSlot, Store};

impl Instance {
    pub(super) fn prepare_managed_recovery(&mut self, store: &Store) -> Result<Vec<AgentSlot>> {
        let profile = self.managed_profile();
        let rows = store.read_slots_for_instance(&self.id)?;
        self.preflight_managed_recovery(&rows)?;
        self.remember_managed_recovery_layout(store, &rows)?;
        self.persist_managed_snapshot()?;
        for index in 0..self.managed_panes.len() {
            if crate::session::managed_pane::recover_pending_slot(&profile, self, index)? {
                self.persist_managed_snapshot()?;
            }
        }
        let rows = store.read_slots_for_instance(&self.id)?;
        for pane in &mut self.managed_panes {
            if rows
                .iter()
                .any(|row| Some(row.slot) == pane.slot && row.tmux_pane.is_empty())
            {
                pane.tmux_pane = None;
            }
        }
        self.persist_managed_snapshot()?;
        self.capture_managed_claude_children()?;
        let mut slots = store.read_slots_for_instance(&self.id)?;
        for index in 0..self.managed_panes.len() {
            let pane = &self.managed_panes[index];
            if pane.tmux_pane.is_some() {
                store.rollback_managed_pane_binding(&self.id, pane)?;
                self.managed_panes[index].tmux_pane = None;
                self.persist_managed_snapshot()?;
            }
        }
        for slot in &mut slots {
            if let Some(previous) = self.managed_recovery_panes.get(&slot.slot) {
                slot.tmux_pane = previous.clone();
            }
        }
        Ok(slots)
    }

    fn remember_managed_recovery_layout(
        &mut self,
        store: &Store,
        rows: &[AgentSlot],
    ) -> Result<()> {
        if !self.managed_recovery_panes.is_empty() {
            if rows.iter().any(|row| row.tmux_pane.is_empty()) {
                return Ok(());
            }
            let Some(snapshot) = store.read_layout_snapshot(&self.id)? else {
                return Ok(());
            };
            let snapshot_panes = crate::tmux::layout::pane_ids(&snapshot.window_layout)?
                .into_iter()
                .collect::<std::collections::HashSet<_>>();
            let current_panes = rows
                .iter()
                .map(|row| row.tmux_pane.clone())
                .collect::<std::collections::HashSet<_>>();
            if snapshot_panes != current_panes {
                return Ok(());
            }
        }
        self.managed_recovery_panes = rows
            .iter()
            .filter(|row| !row.tmux_pane.is_empty())
            .map(|row| (row.slot, row.tmux_pane.clone()))
            .collect();
        Ok(())
    }

    fn preflight_managed_recovery(&self, rows: &[AgentSlot]) -> Result<()> {
        for (&slot, pane) in &self.managed_recovery_panes {
            ensure!(
                (0..=crate::db::MAX_SLOT).contains(&slot),
                "Invalid managed recovery layout slot"
            );
            crate::tmux::managed_pane::validate_tmux_pane(pane)?;
        }
        let profile = self.managed_profile();
        for pane in &self.managed_panes {
            let effective =
                crate::session::managed_pane::effective_slot_record(&profile, self, pane)?;
            let row = rows
                .iter()
                .find(|row| Some(row.slot) == effective.slot)
                .context("Managed recovery slot disappeared")?;
            ensure!(
                row.xats_identity_key == effective.identity_key
                    && row.xats_runtime_generation == effective.slot_generation
                    && (row.tmux_pane.is_empty()
                        || Some(row.tmux_pane.as_str()) == effective.tmux_pane.as_deref()),
                "Managed recovery slot binding changed"
            );
            if !row.tmux_pane.is_empty() {
                crate::session::managed_pane::verify_stored_location(self, &effective)?;
            }
        }
        Ok(())
    }
}
