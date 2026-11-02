use anyhow::{bail, Context, Result};

use super::{Instance, PaneResumeOutcome, RestartMode};
use crate::db::{AgentSlot, Store};

pub(super) struct CodexRestart {
    store: Store,
    slot: AgentSlot,
    verified: bool,
    resume: Option<(String, String)>,
}

impl CodexRestart {
    pub(super) fn begin(
        slot: &AgentSlot,
        mode: RestartMode,
        target: &str,
        live_source: bool,
        session_name: &str,
        working_dir: &str,
    ) -> Result<Option<Self>> {
        if slot.agent != "codex" || !slot.cross_agent_team {
            return Ok(None);
        }
        let store = Store::open_with_schema(&Instance::current_profile())?;
        if live_source && mode == RestartMode::Resume {
            crate::db::codex_capture::verify_live_resume(&store, slot, session_name)?;
        }
        let retiring = if live_source {
            crate::db::codex_capture::current_launch_id(&slot.tmux_pane)?
        } else {
            None
        };
        let verified = store.codex_resume_verified(slot)?;
        let resume = if verified && mode == RestartMode::Resume {
            let endpoint = super::codex_app_server_endpoint().map_err(anyhow::Error::msg)?;
            Some((endpoint.url, working_dir.to_string()))
        } else {
            None
        };
        store.prepare_codex_restart(slot, mode, target, retiring.as_deref())?;
        Ok(Some(Self {
            store,
            slot: slot.clone(),
            verified,
            resume,
        }))
    }

    pub(super) fn prepare_resume(&self) -> Result<()> {
        if let Some((endpoint, working_dir)) = &self.resume {
            crate::codex_permissions::prepare_resume(
                endpoint.clone(),
                self.slot.native_session_id.clone(),
                working_dir.clone(),
                self.slot.yolo_mode,
            )?;
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<()> {
        self.store.finish_codex_restart(&self.slot)
    }

    pub(super) fn abort(self) -> Result<()> {
        self.store.abort_codex_restart(&self.slot, self.verified)
    }
}

impl Instance {
    pub(super) fn resume_recorded_codex_panes(&mut self) -> Result<()> {
        let store = Store::open_with_schema(&Self::current_profile())?;
        let slots = store.read_slots_for_instance(&self.id)?;
        anyhow::ensure!(!slots.is_empty(), "Codex has no tracked panes to resume");
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let primary = crate::tmux::get_agent_pane_id(&session)
            .context("Codex primary pane is unavailable")?;
        anyhow::ensure!(
            slots.iter().any(|slot| slot.slot == 0
                && slot.agent == "codex"
                && slot.cross_agent_team
                && slot.tmux_pane == primary),
            "Tracked Codex primary does not match this session; nothing was restarted"
        );
        let live = crate::db::reconcile::live_session_pane_ids(&session)?;
        anyhow::ensure!(
            slots
                .iter()
                .all(|slot| !slot.tmux_pane.is_empty() && live.contains(&slot.tmux_pane)),
            "Tracked pane is missing or belongs to another session; nothing was restarted"
        );
        for slot in slots
            .iter()
            .filter(|slot| slot.agent == "codex" && slot.cross_agent_team)
        {
            anyhow::ensure!(store.codex_resume_verified(slot)?,
                "Codex slot {} has no verified original thread; use session recover-codex with its original UUID", slot.slot);
            crate::db::codex_capture::verify_live_resume(&store, slot, &session)?;
        }
        let outcomes = self.resume_all_tracked_panes(
            &slots,
            RestartMode::Resume,
            &std::collections::HashMap::new(),
        );
        let errors = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                PaneResumeOutcome::Error(error) => Some(error.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            errors.is_empty(),
            "Codex resume failed: {}",
            errors.join("; ")
        );
        Ok(())
    }

    pub fn recover_codex_thread(&self, profile: &str, slot: i64, thread: &str) -> Result<()> {
        anyhow::ensure!(
            !crate::session::managed_pane::has_managed_state(self),
            "Managed sessions must use their owner-controlled recovery path"
        );
        anyhow::ensure!(!self.is_sandboxed(), "Recovery requires a host Codex pane");
        let store = Store::open_with_schema(profile)?;
        let slots = store.read_slots_for_instance(&self.id)?;
        anyhow::ensure!(
            !slots.iter().any(|record| record.slot != slot
                && record.agent == "codex"
                && record.native_session_id == thread),
            "Another slot already records the selected conversation"
        );
        let slot = slots
            .into_iter()
            .find(|record| record.slot == slot)
            .context("Codex slot not found")?;
        let endpoint = super::codex_app_server_endpoint().map_err(anyhow::Error::msg)?;
        crate::db::codex_capture::recover_explicit_binding(
            &store,
            &slot,
            &crate::tmux::Session::generate_name(&self.id, &self.title),
            &endpoint.url,
            thread,
        )
    }

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
