use anyhow::{bail, Context, Result};

use super::{Instance, PaneResumeOutcome, RestartMode};
use crate::db::{AgentSlot, Store};

pub(super) struct CodexRestart {
    store: Store,
    slot: AgentSlot,
    verified: bool,
}

impl CodexRestart {
    pub(super) fn begin(
        slot: &AgentSlot,
        mode: RestartMode,
        target: &str,
        live_source: bool,
        session_name: &str,
    ) -> Result<Option<Self>> {
        if slot.agent != "codex" || !slot.cross_agent_team {
            return Ok(None);
        }
        let store = Store::open_with_schema(&Instance::current_profile())?;
        if live_source && mode == RestartMode::Resume {
            crate::db::codex_capture::verify_live_resume(slot, session_name)?;
        }
        let retiring = if live_source {
            crate::db::codex_capture::current_launch_id(&slot.tmux_pane)?
        } else {
            None
        };
        let verified = store.codex_resume_verified(slot)?;
        store.prepare_codex_restart(slot, mode, target, retiring.as_deref())?;
        Ok(Some(Self {
            store,
            slot: slot.clone(),
            verified,
        }))
    }

    pub(super) fn finish(self) -> Result<()> {
        self.store.finish_codex_restart(&self.slot)
    }

    pub(super) fn abort(self) -> Result<()> {
        self.store.abort_codex_restart(&self.slot, self.verified)
    }
}

impl Instance {
    pub(super) fn respawn_single_codex(&mut self, mode: RestartMode) -> Result<()> {
        self.ensure_xats_identity_key();
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let pane = crate::tmux::get_agent_pane_id(&session)
            .context("Codex primary pane is unavailable")?;
        let store = Store::open_with_schema(&Self::current_profile())?;
        store.record_launched_slot_config_if_absent(
            &self.id,
            0,
            &self.primary_pane,
            &pane,
            self.xats_identity_key.as_deref().unwrap_or_default(),
            crate::db::now_unix(),
        )?;
        let slot = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|slot| slot.slot == 0 && slot.tmux_pane == pane)
            .context("Codex primary slot does not match its pane")?;
        let outcomes =
            self.resume_all_tracked_panes(&[slot], mode, &std::collections::HashMap::new());
        match outcomes.first() {
            Some(PaneResumeOutcome::Error(error)) => bail!("{error}"),
            Some(_) => Ok(()),
            None => bail!("Codex restart produced no outcome"),
        }
    }
}
