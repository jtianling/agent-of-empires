use anyhow::{ensure, Context, Result};

use super::{Instance, ManagedPane, ManagedSlotTransition};

impl ManagedPane {
    pub(crate) fn stage_slot_transition(&mut self, resume: bool, rebind: bool) -> Result<()> {
        self.validate()?;
        ensure!(
            self.pending_slot_transition.is_none(),
            "Recover the pending slot transition first"
        );
        ensure!(
            self.slot.is_some() && self.slot_generation < crate::db::MAX_XATS_RUNTIME_GENERATION,
            "Managed pane slot cannot advance"
        );
        self.pending_slot_transition = Some(ManagedSlotTransition {
            from_generation: self.slot_generation,
            from_tmux_pane: self.tmux_pane.clone(),
            resume,
            rebind,
            fence: current_fence(self).cloned(),
        });
        self.validate_slot_transition()
    }

    pub(super) fn validate_slot_transition(&self) -> Result<()> {
        let Some(pending) = &self.pending_slot_transition else {
            return Ok(());
        };
        ensure!(
            self.slot.is_some()
                && pending.from_generation == self.slot_generation
                && pending.from_tmux_pane == self.tmux_pane
                && pending.from_generation < crate::db::MAX_XATS_RUNTIME_GENERATION,
            "Managed pending slot transition does not match its record"
        );
        ensure!(
            pending.fence.as_ref() == current_fence(self)
                && (self.xats.is_none() || pending.fence.is_some()),
            "Managed pending slot transition belongs to another launch"
        );
        Ok(())
    }
}

fn current_fence(pane: &ManagedPane) -> Option<&crate::managed_xats::Fence> {
    pane.xats
        .as_ref()
        .and_then(|identity| identity.claim.as_ref())
        .map(|claim| &claim.fence)
}

pub(crate) fn recover_pending_slot(
    profile: &str,
    parent: &mut Instance,
    index: usize,
) -> Result<bool> {
    let pane = parent
        .managed_panes
        .get_mut(index)
        .context("Managed pane disappeared")?;
    if pane.pending_slot_transition.is_none() {
        return Ok(false);
    }
    let store = crate::db::Store::open_with_schema(profile)?;
    recover_with_store(&store, &parent.id, pane)
}

fn recover_with_store(
    store: &crate::db::Store,
    session: &str,
    pane: &mut ManagedPane,
) -> Result<bool> {
    if pane.pending_slot_transition.is_none() {
        return Ok(false);
    }
    let (pending, applied) = read_transition(store, session, pane)?;
    let next_generation = pending.from_generation + 1;
    let next_pane = if pending.rebind {
        None
    } else {
        pending.from_tmux_pane.clone()
    };
    if !applied {
        let prepared =
            store.advance_managed_pane_slot(session, pane, pending.resume, pending.rebind)?;
        ensure!(
            prepared.generation == next_generation,
            "Pending managed slot returned an unexpected generation"
        );
    }
    pane.slot_generation = next_generation;
    pane.tmux_pane = next_pane;
    pane.pending_slot_transition = None;
    Ok(true)
}

pub(crate) fn effective_slot_record(
    profile: &str,
    parent: &Instance,
    pane: &ManagedPane,
) -> Result<ManagedPane> {
    if pane.pending_slot_transition.is_none() {
        return Ok(pane.clone());
    }
    let store = crate::db::Store::open_with_schema(profile)?;
    let (pending, applied) = read_transition(&store, &parent.id, pane)?;
    let mut effective = pane.clone();
    if applied {
        effective.slot_generation = pending.from_generation + 1;
        if pending.rebind {
            effective.tmux_pane = None;
        }
    }
    effective.pending_slot_transition = None;
    Ok(effective)
}

fn read_transition(
    store: &crate::db::Store,
    session: &str,
    pane: &ManagedPane,
) -> Result<(ManagedSlotTransition, bool)> {
    pane.validate()?;
    let pending = pane
        .pending_slot_transition
        .clone()
        .context("Missing slot transition")?;
    let row = store
        .read_slots_for_instance(session)?
        .into_iter()
        .find(|row| Some(row.slot) == pane.slot)
        .context("Pending managed slot disappeared")?;
    ensure!(
        row.xats_identity_key == pane.identity_key,
        "Pending managed slot identity was replaced"
    );
    let applied = row.xats_runtime_generation == pending.from_generation + 1;
    let expected_pane = if applied && pending.rebind {
        ""
    } else {
        pending.from_tmux_pane.as_deref().unwrap_or_default()
    };
    ensure!(
        (applied || row.xats_runtime_generation == pending.from_generation)
            && row.tmux_pane == expected_pane,
        "Pending managed slot was changed by another launch"
    );
    Ok((pending, applied))
}

#[cfg(test)]
mod tests;
