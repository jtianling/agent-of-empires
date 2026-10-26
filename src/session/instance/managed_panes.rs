use anyhow::{ensure, Context, Result};

use super::{ExactSessionRuntimeContext, Instance, RestartMode};
use crate::session::ManagedPane;

impl Instance {
    pub fn primary_pane_id(&self) -> &str {
        self.managed_primary_id.as_deref().unwrap_or(&self.id)
    }

    pub(crate) fn mark_managed_primary(&mut self) -> Result<()> {
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let pane = crate::tmux::managed::primary_pane(&session)?;
        crate::tmux::managed_pane::mark(&session, &pane, self.primary_pane_id())?;
        crate::tmux::managed_pane::clear_ready(&session, &pane, self.primary_pane_id())?;
        self.managed_primary_marked = true;
        Ok(())
    }

    pub(crate) fn prepare_managed_pane_runtime(
        &mut self,
        profile: &str,
        index: usize,
        mode: RestartMode,
    ) -> Result<String> {
        let record = self
            .managed_panes
            .get(index)
            .context("Managed pane disappeared")?
            .clone();
        let mut projected = record.project(self)?;
        projected.source_profile = profile.to_string();
        let result = self.prepare_managed_projection(profile, &record, &mut projected, mode);
        self.managed_panes
            .get_mut(index)
            .context("Managed pane disappeared")?
            .capture_runtime(&projected)?;
        result
    }

    fn prepare_managed_projection(
        &self,
        profile: &str,
        record: &ManagedPane,
        projected: &mut Instance,
        mode: RestartMode,
    ) -> Result<String> {
        ensure!(!record.removing, "Managed pane is being removed");
        let slot = record.slot.context("Managed pane has no reserved slot")?;
        let store = crate::db::Store::open_with_schema(profile)?;
        let row = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|row| row.slot == slot)
            .context("Managed pane slot disappeared")?;
        ensure!(
            row.xats_identity_key == record.identity_key
                && row.xats_runtime_generation == record.slot_generation,
            "Managed pane runtime reservation is stale"
        );
        install_hooks(&record.config.tool)?;
        crate::managed_xats::prepare(projected)?;
        if mode == RestartMode::Fresh {
            projected.begin_fresh_identity(mode);
            crate::managed_codex::mark_fresh(projected)?;
        }
        crate::managed_codex::prepare(projected)?;
        crate::managed_runtimes::prepare(projected)?;
        let exact = self.managed_exact_context(&store, record, mode)?;
        if let Some(runtime) = exact
            .as_ref()
            .filter(|runtime| runtime.shape == crate::agents::ExactSessionRuntime::SharedServer)
        {
            projected.agent_session_id = Some(runtime.native_session_id.clone());
        }
        let resume = (mode == RestartMode::Resume)
            .then_some(row.native_session_id.as_str())
            .filter(|value| !value.is_empty());
        if projected.tool == "shell" {
            return Ok(projected.build_extra_shell_pane_command(&record.config.working_dir));
        }
        projected
            .build_pane_command_with_runtime(
                &record.config,
                resume,
                true,
                Some(&record.identity_key),
                Some(row.model.as_str()).filter(|value| !value.is_empty()),
                exact.as_ref(),
            )
            .context("Could not build managed pane command")
    }

    fn managed_exact_context(
        &self,
        store: &crate::db::Store,
        record: &ManagedPane,
        mode: RestartMode,
    ) -> Result<Option<ExactSessionRuntimeContext>> {
        let Some(shape) = self.pane_exact_session_runtime(&record.config) else {
            return Ok(None);
        };
        let slot = record.slot.context("Managed pane has no slot")?;
        let existing = store
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|row| row.slot == slot)
            .context("Managed pane slot disappeared")?;
        let mut runtime = ExactSessionRuntimeContext {
            shape,
            server_base_url: String::new(),
            slot,
            generation: record.slot_generation,
            native_session_id: if mode == RestartMode::Resume {
                existing.native_session_id
            } else {
                String::new()
            },
            identity_key: record.identity_key.clone(),
        };
        if shape == crate::agents::ExactSessionRuntime::SharedServer {
            self.prepare_shared_server_session(
                store,
                &record.config,
                record.tmux_pane.as_deref().unwrap_or_default(),
                &mut runtime,
                mode,
            )?;
        }
        Ok(Some(runtime))
    }
}

fn install_hooks(tool: &str) -> Result<()> {
    if let Some(config) =
        crate::agents::get_agent(tool).and_then(|agent| agent.hook_config.as_ref())
    {
        let home = dirs::home_dir().context("Missing home for pane hooks")?;
        crate::hooks::install_hooks(&home.join(config.settings_rel_path), config.events, tool)
            .context("Installing managed pane hooks")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_projection_uses_parent_slot_and_logical_pane() {
        let mut parent = Instance::new("parent", "/tmp");
        parent.source_profile = "testing".into();
        let mut pane = ManagedPane::new(
            uuid::Uuid::new_v4().to_string(),
            crate::session::PaneConfig::new("opencode", "/tmp", false, false),
            None,
        )
        .unwrap();
        pane.slot = Some(2);
        pane.slot_generation = 4;
        let projected = pane.project(&parent).unwrap();
        let runtime = ExactSessionRuntimeContext {
            shape: crate::agents::ExactSessionRuntime::OwnedServer,
            server_base_url: String::new(),
            slot: 2,
            generation: 4,
            native_session_id: String::new(),
            identity_key: pane.identity_key.clone(),
        };
        let command = projected
            .build_opencode_runtime_command(&pane.config, None, true, Some(&runtime))
            .unwrap();
        let words = shell_words::split(&command).unwrap();
        let argument = |name| words.windows(2).find(|pair| pair[0] == name).unwrap()[1].clone();
        assert_eq!(argument("--instance-id"), parent.id);
        assert_eq!(argument("--managed-pane-id"), pane.id);
        assert_eq!(argument("--slot"), "2");
        assert_eq!(argument("--generation"), "4");
        assert_eq!(argument("-p"), "testing");
    }

    #[test]
    fn human_child_projection_does_not_inherit_a_new_parent_owner() {
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(uuid::Uuid::new_v4().to_string());
        let mut pane = ManagedPane::new(
            uuid::Uuid::new_v4().to_string(),
            crate::session::PaneConfig::new("claude", "/tmp", false, false),
            None,
        )
        .unwrap();
        pane.owner = None;
        pane.slot = Some(1);
        let projection = pane.project(&parent).unwrap();
        assert!(projection.managed_owner.is_none());
        assert_eq!(
            projection.managed_pane_target.unwrap().session_id,
            parent.id
        );
        assert_eq!(projection.id, pane.id);
    }

    #[test]
    fn original_opencode_primary_is_targeted_but_human_extra_is_not_claimed() {
        let mut parent = Instance::new("parent", "/tmp");
        parent.managed_owner = Some(uuid::Uuid::new_v4().to_string());
        let pane = crate::session::PaneConfig::new("opencode", "/tmp", false, false);
        let primary = parent
            .build_opencode_runtime_command(&pane, None, true, None)
            .unwrap();
        let words = shell_words::split(&primary).unwrap();
        assert!(words
            .windows(2)
            .any(|pair| pair[0] == "--managed-pane-id" && pair[1] == parent.id));
        let extra = parent
            .build_opencode_runtime_command(&pane, None, false, None)
            .unwrap();
        assert!(!shell_words::split(&extra)
            .unwrap()
            .iter()
            .any(|word| word == "--managed-pane-id"));
    }
}
