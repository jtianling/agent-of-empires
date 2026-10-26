use anyhow::{ensure, Context, Result};

use super::{Instance, RestartMode, SessionLaunch};
use crate::session::{Group, GroupTree, ManagedPane, Storage};

impl Instance {
    pub(super) fn persist_managed_snapshot(&self) -> Result<()> {
        let storage = Storage::new(&self.managed_profile())?;
        let (mut instances, groups) = storage.load_with_groups()?;
        persist(&storage, &mut instances, &groups, self)
    }

    pub(crate) fn relaunch_managed_extra(
        &mut self,
        pane_id: &str,
        mode: RestartMode,
        replacement: Option<&str>,
    ) -> Result<()> {
        let profile = self.managed_profile();
        let storage = Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        let parent = instances
            .iter()
            .find(|instance| instance.id == self.id)
            .context("Managed session disappeared")?;
        *self = parent.clone();
        self.source_profile = profile;
        ensure!(
            self.managed_primary_replacement.is_none(),
            "Managed primary replacement is incomplete"
        );
        let index = self
            .managed_panes
            .iter()
            .position(|pane| pane.id == pane_id)
            .context("Managed pane disappeared")?;
        if crate::session::managed_pane::recover_pending_slot(storage.profile(), self, index)? {
            persist(&storage, &mut instances, &groups, self)?;
        }
        let mut outcome = self.relaunch_managed_record(
            &storage,
            &mut instances,
            &groups,
            index,
            mode,
            replacement,
        );
        if let (Err(error), Some(target)) = (&outcome, replacement) {
            if let Err(cleanup) =
                self.rollback_managed_replacement(storage.profile(), index, target)
            {
                outcome = Err(anyhow::anyhow!(
                    "{error:#}; pane rollback failed: {cleanup:#}"
                ));
            }
        }
        persist(&storage, &mut instances, &groups, self)?;
        outcome
    }

    fn rollback_managed_replacement(
        &mut self,
        profile: &str,
        index: usize,
        target: &str,
    ) -> Result<()> {
        let record = &self.managed_panes[index];
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        if let Some(location) = crate::tmux::managed_pane::locate(target)? {
            ensure!(
                location.session == session
                    && location
                        .marker
                        .as_deref()
                        .map_or(true, |id| id == record.id),
                "Recovery pane was replaced before rollback"
            );
            crate::tmux::kill_pane_exact(target)?;
        }
        let store = crate::db::Store::open_with_schema(profile)?;
        let row = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|row| Some(row.slot) == record.slot)
            .context("Recovery slot disappeared")?;
        ensure!(
            row.xats_identity_key == record.identity_key
                && row.xats_runtime_generation == record.slot_generation,
            "Recovery slot changed before rollback"
        );
        if row.tmux_pane == target {
            let mut bound = record.clone();
            bound.tmux_pane = Some(target.to_string());
            store.rollback_managed_pane_binding(&self.id, &bound)?;
            self.managed_panes[index].tmux_pane = None;
        } else if row.tmux_pane.is_empty() {
            self.managed_panes[index].tmux_pane = None;
        }
        Ok(())
    }

    fn relaunch_managed_record(
        &mut self,
        storage: &Storage,
        instances: &mut [Instance],
        groups: &[Group],
        index: usize,
        requested: RestartMode,
        replacement: Option<&str>,
    ) -> Result<()> {
        let record = self.managed_panes[index].clone();
        ensure!(!record.removing, "Managed pane removal is incomplete");
        let store = crate::db::Store::open_with_schema(storage.profile())?;
        let row = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|row| Some(row.slot) == record.slot)
            .context("Managed pane slot disappeared")?;
        let mode = resume_mode(&record, &row.native_session_id, requested);
        self.validate_managed_relaunch(storage.profile(), &record, replacement)?;
        let mut projection = record.project(self)?;
        if mode == RestartMode::Resume
            && !row.native_session_id.is_empty()
            && !(record.config.tool == "codex" && record.xats.is_some())
        {
            projection.agent_session_id = Some(row.native_session_id.clone());
        }
        crate::managed_xats::stage_restart(&mut projection)?;
        self.managed_panes[index].capture_runtime(&projection)?;
        persist(storage, instances, groups, self)?;
        let prepared = crate::managed_xats::prepare(&mut projection);
        self.managed_panes[index].capture_runtime(&projection)?;
        persist(storage, instances, groups, self)?;
        prepared?;
        if replacement.is_none() {
            let target = record
                .tmux_pane
                .as_deref()
                .context("Managed pane is not bound")?;
            let session = crate::tmux::Session::generate_name(&self.id, &self.title);
            crate::tmux::managed_pane::clear_ready(&session, target, &record.id)?;
            self.stop_managed_pane(target)?;
        }
        self.managed_panes[index]
            .stage_slot_transition(mode == RestartMode::Resume, replacement.is_some())?;
        persist(storage, instances, groups, self)?;
        crate::session::managed_pane::recover_pending_slot(storage.profile(), self, index)?;
        persist(storage, instances, groups, self)?;
        let built = self.prepare_managed_pane_runtime(storage.profile(), index, mode);
        persist(storage, instances, groups, self)?;
        let command = built?;
        self.bind_and_respawn_managed(storage, instances, groups, index, replacement, &command)
    }

    fn validate_managed_relaunch(
        &self,
        profile: &str,
        record: &ManagedPane,
        replacement: Option<&str>,
    ) -> Result<()> {
        if replacement.is_none() {
            crate::session::managed_pane::verify_binding(profile, self, record)?;
            return Ok(());
        }
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let live = crate::tmux::managed_pane::live_panes(&session)?;
        ensure!(
            !live
                .iter()
                .any(|pane| pane.marker.as_deref() == Some(&record.id)
                    || (record.tmux_pane.as_deref() == Some(pane.tmux_pane.as_str())
                        && Some(pane.tmux_pane.as_str()) != replacement)),
            "Managed pane is still live or its previous target was replaced"
        );
        Ok(())
    }

    fn bind_and_respawn_managed(
        &mut self,
        storage: &Storage,
        instances: &mut [Instance],
        groups: &[Group],
        index: usize,
        replacement: Option<&str>,
        command: &str,
    ) -> Result<()> {
        let record = &self.managed_panes[index];
        let pane = replacement
            .or(record.tmux_pane.as_deref())
            .context("Managed pane target is missing")?
            .to_string();
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        if replacement.is_some() {
            let store = crate::db::Store::open_with_schema(storage.profile())?;
            let row = store
                .read_slots_for_instance(&self.id)?
                .into_iter()
                .find(|row| Some(row.slot) == record.slot)
                .context("Managed pane slot disappeared")?;
            store.bind_prepared_slot_pane(
                &self.id,
                &record.config.tool,
                row.slot,
                record.slot_generation,
                &record.identity_key,
                &row.native_session_id,
                &pane,
                crate::db::now_unix(),
            )?;
            crate::tmux::managed_pane::mark(&session, &pane, &record.id)?;
            self.managed_panes[index].tmux_pane = Some(pane.clone());
        }
        persist(storage, instances, groups, self)?;
        let record = &self.managed_panes[index];
        crate::tmux::managed_pane::clear_ready(&session, &pane, &record.id)?;
        crate::tmux::respawn_pane_target(
            &pane,
            command,
            &record.config.working_dir,
            record.config.tool != "shell",
        )?;
        if ready_after_respawn(record) {
            crate::tmux::managed_pane::mark_ready(
                &session,
                &pane,
                &record.id,
                record.slot_generation,
            )?;
        }
        self.auto_confirm_launched_pane(&pane, &record.config);
        Ok(())
    }

    pub(super) fn start_managed_container(
        &mut self,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
        mode: RestartMode,
    ) -> Result<()> {
        let profile = self.managed_profile();
        let storage = Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        *self = instances
            .iter()
            .find(|instance| instance.id == self.id)
            .context("Managed session disappeared")?
            .clone();
        self.source_profile = profile;
        ensure!(
            self.managed_primary_replacement.is_none(),
            "Managed primary replacement is incomplete"
        );
        if self.tmux_session()?.exists() {
            return Ok(());
        }
        let result = self.start_with_size_inner(size, skip_on_launch, SessionLaunch::Agent, mode);
        persist(&storage, &mut instances, &groups, self)?;
        result?;
        self.restore_managed_extras(mode)
    }

    pub(super) fn restart_managed_container(
        &mut self,
        size: Option<(u16, u16)>,
        mode: RestartMode,
    ) -> Result<()> {
        let profile = self.managed_profile();
        let storage = Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        *self = instances
            .iter()
            .find(|instance| instance.id == self.id)
            .context("Managed session disappeared")?
            .clone();
        self.source_profile = profile;
        ensure!(
            self.managed_primary_replacement.is_none(),
            "Managed primary replacement is incomplete"
        );
        if !self.tmux_session()?.exists() {
            return self.start_managed_container(size, false, mode);
        }
        let snapshot = self.begin_fresh_identity(mode);
        self.mark_managed_primary()?;
        let result = self.respawn_single_pane_inner(mode);
        self.rollback_fresh_identity_on_failure(snapshot, result.is_ok());
        persist(&storage, &mut instances, &groups, self)?;
        result
    }

    pub(super) fn restore_managed_extras(&mut self, mode: RestartMode) -> Result<()> {
        let ids: Vec<_> = self
            .managed_panes
            .iter()
            .filter(|record| !record.removing && record.slot.is_some())
            .map(|record| record.id.clone())
            .collect();
        let mut errors = Vec::new();
        for id in ids {
            if let Err(error) = self.restore_managed_extra(&id, mode) {
                errors.push(format!("{id}: {error:#}"));
            }
        }
        ensure!(
            errors.is_empty(),
            "Managed pane recovery failed: {}",
            errors.join("; ")
        );
        Ok(())
    }

    fn restore_managed_extra(&mut self, id: &str, mode: RestartMode) -> Result<()> {
        self.recover_managed_slot_state(id)?;
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let record = self
            .managed_panes
            .iter()
            .find(|record| record.id == id)
            .context("Managed pane disappeared")?
            .clone();
        let live = crate::tmux::managed_pane::live_panes(&session)?;
        if live
            .iter()
            .any(|pane| record.tmux_pane.as_deref() == Some(pane.tmux_pane.as_str()))
        {
            crate::session::managed_pane::verify_binding(&self.managed_profile(), self, &record)?;
            return Ok(());
        }
        let placeholder = self.build_extra_shell_pane_command(&record.config.working_dir);
        let pane = crate::tmux::split_window_right(
            &session,
            &record.config.working_dir,
            &placeholder,
            false,
        )?;
        if let Err(error) = self.relaunch_managed_extra(id, mode, Some(&pane)) {
            let recorded = self
                .managed_panes
                .iter()
                .find(|record| record.id == id)
                .is_some_and(|record| record.tmux_pane.as_deref() == Some(&pane));
            if !recorded {
                if let Some(location) = crate::tmux::managed_pane::locate(&pane)? {
                    ensure!(
                        location.session == session
                            && location
                                .marker
                                .as_deref()
                                .map_or(true, |marker| marker == id),
                        "Recovery placeholder changed"
                    );
                    crate::tmux::kill_pane_exact(&pane).with_context(|| format!("{error:#}"))?;
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn recover_managed_slot_state(&mut self, id: &str) -> Result<()> {
        if !self
            .managed_panes
            .iter()
            .any(|pane| pane.id == id && pane.pending_slot_transition.is_some())
        {
            return Ok(());
        }
        let profile = self.managed_profile();
        let storage = Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        *self = instances
            .iter()
            .find(|parent| parent.id == self.id)
            .context("Managed session disappeared")?
            .clone();
        self.source_profile = profile;
        let index = self
            .managed_panes
            .iter()
            .position(|pane| pane.id == id)
            .context("Managed pane disappeared")?;
        if crate::session::managed_pane::recover_pending_slot(storage.profile(), self, index)? {
            persist(&storage, &mut instances, &groups, self)?;
        }
        Ok(())
    }

    fn managed_profile(&self) -> String {
        if self.source_profile.is_empty() {
            Self::current_profile()
        } else {
            self.source_profile.clone()
        }
    }
}

fn persist(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    instance: &Instance,
) -> Result<()> {
    *instances
        .iter_mut()
        .find(|parent| parent.id == instance.id)
        .context("Managed parent disappeared")? = instance.clone();
    storage.save_with_groups(instances, &GroupTree::new_with_groups(instances, groups))
}

fn resume_mode(record: &ManagedPane, native: &str, mode: RestartMode) -> RestartMode {
    if mode == RestartMode::Fresh {
        return mode;
    }
    let resumable = match record.config.tool.as_str() {
        "claude" => {
            uuid::Uuid::try_parse(native).is_ok()
                && record.native_session_id.as_deref() == Some(native)
        }
        "codex" if record.xats.is_some() => record
            .xats
            .as_ref()
            .and_then(|xats| xats.client_config.as_ref())
            .and_then(|value| value.get("thread_id"))
            .and_then(|value| value.as_str())
            .is_some_and(|id| uuid::Uuid::try_parse(id).is_ok()),
        "codex" => uuid::Uuid::try_parse(native).is_ok(),
        "opencode" => crate::opencode_runtime::validate_session_id(native).is_ok(),
        "kimi" => crate::kimi::validate_session_id(native).is_ok(),
        _ => false,
    };
    if resumable {
        RestartMode::Resume
    } else {
        RestartMode::Fresh
    }
}

fn ready_after_respawn(record: &ManagedPane) -> bool {
    record.config.tool != "opencode"
        && (record.xats.is_none() || !matches!(record.config.tool.as_str(), "claude" | "codex"))
}
