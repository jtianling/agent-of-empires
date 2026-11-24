use anyhow::{ensure, Context, Result};

use super::{Instance, PaneResumeOutcome, RestartMode};
use crate::db::{AgentSlot, Store};
use crate::session::Storage;

fn check_resume_slots(slots: &[AgentSlot], mode: RestartMode) -> Result<()> {
    if mode == RestartMode::Resume {
        for slot in slots.iter().filter(|s| s.agent == "antigravity") {
            ensure!(
                crate::hooks::antigravity::valid_id(&slot.native_session_id),
                "Antigravity slot {} has no verified conversation UUID",
                slot.slot
            );
        }
    }
    Ok(())
}

impl Instance {
    pub(super) fn preflight_antigravity(
        &self,
        slots: &[AgentSlot],
        mode: RestartMode,
    ) -> Result<()> {
        check_resume_slots(slots, mode)?;
        for slot in slots.iter().filter(|s| s.agent == "antigravity") {
            crate::hooks::antigravity::install_for(&slot.pane_config(), self.is_sandboxed())?;
        }
        Ok(())
    }

    pub(super) fn respawn_single_antigravity(&mut self, mode: RestartMode) -> Result<()> {
        let profile = self.managed_profile();
        let _operation = Storage::new(&profile)?.operation_lock()?;
        let store = Store::open_with_schema(&profile)?;
        if mode == RestartMode::Resume {
            let slots = store.read_slots_for_instance(&self.id)?;
            let primary = slots
                .iter()
                .find(|s| s.slot == 0 && s.agent == "antigravity")
                .context("Antigravity primary pane has no matching durable slot")?;
            check_resume_slots(std::slice::from_ref(primary), mode)?;
        }
        let name = crate::tmux::Session::generate_name(&self.id, &self.title);
        let pane = crate::tmux::get_agent_pane_id(&name)
            .context("Antigravity primary pane is unavailable")?;
        if mode == RestartMode::Fresh {
            self.ensure_xats_identity_key();
            store.record_launched_slot_config_if_absent(
                &self.id,
                0,
                &self.primary_pane,
                &pane,
                self.xats_identity_key.as_deref().unwrap_or_default(),
                crate::db::now_unix(),
            )?;
        }
        let slot = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|s| s.slot == 0 && s.agent == "antigravity" && s.tmux_pane == pane)
            .context("Antigravity primary pane has no matching durable slot")?;
        check_resume_slots(std::slice::from_ref(&slot), mode)?;
        let mut slots = [slot];
        let keys = self.ensure_slot_identity_keys(&store, &mut slots);
        outcomes(self.resume_all_tracked_panes(&slots, mode, &keys))
    }

    pub(super) fn restart_antigravity_slots(
        &mut self,
        mode: RestartMode,
        start_only: bool,
    ) -> Result<bool> {
        if self.is_sandboxed() || crate::session::managed_pane::has_managed_state(self) {
            return Ok(false);
        }
        let profile = self.managed_profile();
        if self.tool != "antigravity" && !Store::has_antigravity_slots(&profile, &self.id)? {
            return Ok(false);
        }
        let _operation = Storage::new(&profile)?.operation_lock()?;
        let store = Store::open_with_schema(&profile)?;
        let read = store.read_slots_for_instance_with_diagnostics(&self.id)?;
        ensure!(
            read.skipped == 0,
            "Cannot restart with invalid durable slots"
        );
        let mut slots = read.slots;
        if !slots.iter().any(|s| s.agent == "antigravity") {
            ensure!(
                mode != RestartMode::Resume
                    || self.tool != "antigravity"
                    || (start_only
                        && slots.is_empty()
                        && self.status != crate::session::Status::Stopped),
                "Antigravity session has no tracked conversation to resume"
            );
            return Ok(false);
        }
        if !start_only {
            check_resume_slots(&slots, mode)?;
        }
        let session = self.tmux_session()?;
        let live = session.exists();
        if live && start_only {
            return Ok(true);
        }
        if start_only {
            check_resume_slots(&slots, mode)?;
        }
        if live {
            let name = crate::tmux::Session::generate_name(&self.id, &self.title);
            let primary =
                crate::tmux::get_agent_pane_id(&name).context("Primary pane is unavailable")?;
            let panes = crate::db::reconcile::live_session_pane_ids(&name)?;
            ensure!(
                slots.iter().any(|s| s.slot == 0 && s.tmux_pane == primary)
                    && slots.iter().all(|s| panes.contains(&s.tmux_pane)),
                "Tracked slots no longer match the live panes"
            );
        }
        let keys = self.ensure_slot_identity_keys(&store, &mut slots);
        drop(_operation);
        let results = if live {
            self.resume_all_tracked_panes(&slots, mode, &keys)
        } else {
            self.recover_from_slots(&store, &slots, mode, &keys)?
        };
        outcomes(results)?;
        Ok(true)
    }
}

fn outcomes(results: Vec<PaneResumeOutcome>) -> Result<()> {
    ensure!(!results.is_empty(), "No Antigravity panes launched");
    let errors = results
        .into_iter()
        .filter_map(|result| match result {
            PaneResumeOutcome::Error(error) => Some(error),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        errors.is_empty(),
        "Pane restart failed: {}",
        errors.join("; ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{PaneConfig, Status};

    #[test]
    #[serial_test::serial]
    fn antigravity_cli_restart_and_cold_resume_require_slot_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(temp.path());
        let profile = "test";
        let mut inst = Instance::new("test", "/same");
        let pane = PaneConfig::new("antigravity", "/same", false, false);
        inst.set_primary_pane_config(pane.clone());
        inst.source_profile = profile.into();
        inst.status = Status::Stopped;
        inst.resume_token = Some("2f52b5ab-0c1f-4121-9150-4595b571f562".into());
        assert!(inst
            .start_with_size(None)
            .unwrap_err()
            .to_string()
            .contains("no tracked conversation"));
        assert!(inst
            .restart_antigravity_slots(RestartMode::Resume, true)
            .is_err());
        let store = Store::open_with_schema(profile).unwrap();
        store
            .record_launched_slot_config(&inst.id, 0, &pane, "%61", "", 1)
            .unwrap();
        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert!(inst
            .respawn_agent_pane()
            .unwrap_err()
            .to_string()
            .contains("verified conversation UUID"));
        assert!(inst
            .restart_with_size(None)
            .unwrap_err()
            .to_string()
            .contains("verified conversation UUID"));
        let error = inst
            .restart_antigravity_slots(RestartMode::Resume, false)
            .unwrap_err();
        assert!(error.to_string().contains("verified conversation UUID"));
        assert!(inst
            .preflight_antigravity(&slots, RestartMode::Resume)
            .is_err());
        assert!(inst
            .slot_resume_source(&slots[0], RestartMode::Resume)
            .is_empty());
        let claude = PaneConfig::new("claude", "/same", false, false);
        inst.set_primary_pane_config(claude.clone());
        store
            .record_launched_slot_config(&inst.id, 0, &claude, "%61", "", 1)
            .unwrap();
        store
            .record_launched_slot_config(&inst.id, 1, &pane, "%62", "", 1)
            .unwrap();
        let error = inst.restart_with_size(None).unwrap_err();
        assert!(error.to_string().contains("Antigravity slot 1"));
    }

    #[test]
    #[serial_test::serial]
    fn antigravity_fresh_restart_does_not_require_conversation_uuid_or_existing_slots() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::session::scoped_test_home(temp.path());
        let profile = "fresh-test";
        let mut inst = Instance::new("test", "/same");
        let pane = PaneConfig::new("antigravity", "/same", false, true);
        inst.set_primary_pane_config(pane.clone());
        inst.source_profile = profile.into();
        inst.status = Status::Stopped;
        assert!(!inst
            .restart_antigravity_slots(RestartMode::Fresh, false)
            .unwrap());

        let store = Store::open_with_schema(profile).unwrap();
        let key = uuid::Uuid::new_v4().to_string();
        store
            .record_launched_slot_config(&inst.id, 0, &pane, "%71", &key, 1)
            .unwrap();
        let slots = store.read_slots_for_instance(&inst.id).unwrap();
        assert!(slots[0].native_session_id.is_empty());
        assert!(inst
            .preflight_antigravity(&slots, RestartMode::Fresh)
            .is_ok());
        assert_eq!(slots[0].xats_identity_key, key);
    }
}
