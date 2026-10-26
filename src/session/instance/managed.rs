use anyhow::{ensure, Context, Result};

use super::{Instance, RestartMode, SessionLaunch, Status, PANE_EXIT_ATTEMPTS, PANE_EXIT_POLL};
use crate::session::{Group, GroupTree, Storage};

impl Instance {
    pub(super) fn managed_lifecycle(
        &mut self,
        restart: Option<RestartMode>,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
    ) -> Result<()> {
        let profile = if self.source_profile.is_empty() {
            Self::current_profile()
        } else {
            self.source_profile.clone()
        };
        let storage = Storage::new(&profile)?;
        let _operation = storage.operation_lock()?;
        let (mut instances, groups) = storage.load_with_groups()?;
        let index = instances
            .iter()
            .position(|instance| instance.id == self.id)
            .context("Managed session no longer exists")?;
        ensure!(
            instances[index].managed_xats.is_some(),
            "Managed identity was removed"
        );
        *self = instances[index].clone();
        ensure!(
            self.managed_primary_replacement.is_none(),
            "Managed primary replacement must finish before launch"
        );
        self.source_profile = profile;
        let running = self.tmux_session()?.exists();
        if restart.is_none() && running {
            return Ok(());
        }
        let outcome = self.managed_transition(
            &storage,
            &mut instances,
            &groups,
            restart,
            running,
            size,
            skip_on_launch,
        );
        if let Err(error) = &outcome {
            self.status = Status::Error;
            self.last_error = Some(format!("{error:#}"));
        }
        persist(&storage, &mut instances, &groups, self)?;
        outcome?;
        if !running {
            self.restore_managed_extras(restart.unwrap_or(RestartMode::Resume))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn managed_transition(
        &mut self,
        storage: &Storage,
        instances: &mut [Instance],
        groups: &[Group],
        restart: Option<RestartMode>,
        running: bool,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
    ) -> Result<()> {
        let native = crate::db::Store::open_with_schema(storage.profile())?
            .read_slots_for_instance(&self.id)?
            .into_iter()
            .find(|slot| slot.slot == 0 && slot.agent == self.tool)
            .map(|slot| slot.native_session_id);
        let mode = launch_mode(self, restart, native.as_deref());
        crate::managed_xats::stage_restart(self)?;
        persist(storage, instances, groups, self)?;
        crate::managed_xats::prepare(self)?;
        persist(storage, instances, groups, self)?;
        if running {
            self.stop_managed_primary()?;
        }
        if mode == RestartMode::Fresh {
            self.begin_fresh_identity(mode);
            crate::managed_codex::mark_fresh(self)?;
        }
        persist(storage, instances, groups, self)?;
        if running {
            crate::managed_codex::prepare(self)?;
            crate::managed_runtimes::prepare(self)?;
            persist(storage, instances, groups, self)?;
            self.mark_managed_primary()?;
            if self.tool == "claude" && mode == RestartMode::Resume {
                self.resume_token = self.agent_session_id.clone();
            }
            self.respawn_single_pane_inner(mode)
        } else {
            self.start_with_size_inner(size, skip_on_launch, SessionLaunch::Agent, mode)
        }
    }

    fn stop_managed_primary(&self) -> Result<()> {
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        let pane = crate::tmux::managed::primary_pane(&session)?;
        self.stop_managed_pane(&pane)
    }

    pub(super) fn stop_managed_pane(&self, pane: &str) -> Result<()> {
        let session = crate::tmux::Session::generate_name(&self.id, &self.title);
        crate::tmux::set_pane_remain_on_exit(pane, true)?;
        crate::tmux::managed::verify_owner(&session, pane)?;
        let original_pid = crate::process::get_pane_pid(pane);
        crate::tmux::kill_pane_process_tree_target(pane);
        for _ in 0..PANE_EXIT_ATTEMPTS {
            let current_pid = crate::process::get_pane_pid(pane);
            if primary_process_exited(
                original_pid,
                current_pid,
                crate::tmux::utils::is_pane_dead(pane),
            ) {
                crate::tmux::managed::verify_owner(&session, pane)?;
                return Ok(());
            }
            std::thread::sleep(PANE_EXIT_POLL);
        }
        anyhow::bail!("Managed primary pane did not exit before restart")
    }
}

fn primary_process_exited(original: Option<u32>, current: Option<u32>, dead: bool) -> bool {
    dead || current.is_none() || current != original
}

fn persist(
    storage: &Storage,
    instances: &mut [Instance],
    groups: &[Group],
    instance: &Instance,
) -> Result<()> {
    let target = instances
        .iter_mut()
        .find(|target| target.id == instance.id)
        .context("Managed session disappeared")?;
    *target = instance.clone();
    storage.save_with_groups(instances, &GroupTree::new_with_groups(instances, groups))
}

fn launch_mode(
    instance: &Instance,
    requested: Option<RestartMode>,
    native: Option<&str>,
) -> RestartMode {
    if requested == Some(RestartMode::Fresh) {
        return RestartMode::Fresh;
    }
    let resumable = match instance.tool.as_str() {
        "claude" => native.is_some_and(|id| {
            uuid::Uuid::parse_str(id).is_ok() && instance.agent_session_id.as_deref() == Some(id)
        }),
        "opencode" => {
            native.is_some_and(|id| crate::opencode_runtime::validate_session_id(id).is_ok())
        }
        "codex" => instance.managed_xats.as_ref().is_some_and(|identity| {
            identity
                .client_config
                .as_ref()
                .and_then(|config| config.get("thread_id"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        }),
        _ => false,
    };
    if resumable {
        RestartMode::Resume
    } else {
        RestartMode::Fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_fallback_does_not_keep_the_old_agent_alive() {
        assert!(primary_process_exited(Some(100), Some(200), false));
        assert!(primary_process_exited(Some(100), Some(100), true));
        assert!(primary_process_exited(Some(100), None, false));
        assert!(!primary_process_exited(Some(100), Some(100), false));
    }

    #[test]
    fn preallocated_claude_id_does_not_prove_a_conversation_exists() {
        let mut instance = Instance::new("worker", "/tmp");
        let id = uuid::Uuid::new_v4().to_string();
        instance.tool = "claude".into();
        instance.agent_session_id = Some(id.clone());
        assert_eq!(launch_mode(&instance, None, None), RestartMode::Fresh);
        assert_eq!(
            launch_mode(&instance, Some(RestartMode::Resume), None),
            RestartMode::Fresh
        );
        assert_eq!(launch_mode(&instance, None, Some("")), RestartMode::Fresh);
        assert_eq!(launch_mode(&instance, None, Some(&id)), RestartMode::Resume);
        assert_eq!(
            launch_mode(&instance, Some(RestartMode::Fresh), Some(&id)),
            RestartMode::Fresh
        );
        let stale = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            launch_mode(&instance, None, Some(&stale)),
            RestartMode::Fresh
        );
    }
}
