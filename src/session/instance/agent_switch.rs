use anyhow::{bail, ensure, Result};

use super::Instance;
use crate::session::Config;

impl Instance {
    pub(super) fn finish_runtime_handoffs(
        &self,
        slots: &[crate::db::AgentSlot],
        outcomes: &mut [super::PaneResumeOutcome],
    ) {
        for (slot, outcome) in slots.iter().zip(outcomes.iter_mut()) {
            if matches!(outcome, super::PaneResumeOutcome::Error(_)) {
                continue;
            }
            if let Err(error) =
                crate::runtime_handoff::wait_committed(&self.managed_profile(), &self.id, slot.slot)
            {
                tracing::error!(
                    slot = slot.slot,
                    "Runtime identity handoff failed: {error:#}"
                );
                *outcome = super::PaneResumeOutcome::Error(format!("{error:#}"));
            }
        }
    }

    pub(crate) fn switch_pane_agent(
        &mut self,
        slot: i64,
        target: &str,
        config: &Config,
    ) -> Result<()> {
        ensure!(
            crate::agents::is_switchable_agent(target),
            "'{}' is not a switchable agent",
            target
        );
        ensure!(
            self.managed_primary_replacement.is_none(),
            "Managed primary replacement is incomplete"
        );
        if slot == 0 {
            return self.switch_agent(target, config);
        }
        let Some(index) = self
            .managed_panes
            .iter()
            .position(|pane| pane.slot == Some(slot))
        else {
            return Ok(());
        };
        let pane = &self.managed_panes[index];
        ensure!(
            !pane.removing && pane.pending_slot_transition.is_none(),
            "Managed pane transition is incomplete"
        );
        let pane = &mut self.managed_panes[index];
        reset_runtime(&mut pane.xats, &pane.config.tool);
        pane.config.tool = target.to_string();
        pane.native_session_id = None;
        Ok(())
    }

    /// Hand this instance's primary pane to `target`, discarding the old
    /// agent's conversation but keeping its xats identity key.
    ///
    /// Command and extra args are rebuilt the way a new session of `target`
    /// would get them: both describe the old agent's binary.
    pub fn switch_agent(&mut self, target: &str, config: &Config) -> Result<()> {
        if !crate::agents::is_switchable_agent(target) {
            bail!("'{}' is not a switchable agent", target);
        }
        let mut pane = self.primary_pane.clone();
        reset_runtime(&mut self.managed_xats, &pane.tool);
        pane.tool = target.to_string();
        self.set_primary_pane_config(pane);

        let default_command = crate::agents::get_agent(target)
            .filter(|agent| agent.set_default_command)
            .map(|agent| agent.binary.to_string())
            .unwrap_or_default();
        self.command = config
            .session
            .agent_command_override
            .get(target)
            .filter(|command| !command.is_empty())
            .cloned()
            .unwrap_or(default_command);
        self.extra_args = config
            .session
            .agent_extra_args
            .get(target)
            .cloned()
            .unwrap_or_default();

        self.resume_token = None;
        self.agent_session_id = None;
        self.fork_pending = None;
        Ok(())
    }
}

fn reset_runtime(identity: &mut Option<crate::managed_xats::ManagedXats>, source: &str) {
    if let Some(identity) = identity {
        if source == "codex" && identity.retiring_codex.is_none() {
            if let Some(client_config) = identity.client_config.take() {
                identity.retiring_codex = Some(crate::managed_xats::RetiringCodex {
                    client_config,
                    launch_instance_id: identity.launch_instance_id.clone(),
                    fence: identity.claim.as_ref().map(|claim| claim.fence.clone()),
                });
            }
        }
        if identity.claim.is_some() && identity.previous_agent_type.is_none() {
            identity.previous_agent_type = Some(match source {
                "claude" => "claude-code".to_string(),
                source => source.to_string(),
            });
        }
        identity.model = None;
        identity.effort = None;
        identity.client_config = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_instance() -> Instance {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst.sync_primary_pane_from_legacy();
        inst.xats_identity_key = Some("key-0".to_string());
        inst.resume_token = Some("resume".to_string());
        inst.agent_session_id = Some("sess".to_string());
        inst.fork_pending = Some("fork".to_string());
        inst.extra_args = "--claude-only".to_string();
        inst
    }

    #[test]
    fn claude_to_codex_resets_launch_and_keeps_identity() {
        let mut inst = claude_instance();
        inst.switch_agent("codex", &Config::default()).unwrap();

        assert_eq!(inst.tool, "codex");
        assert_eq!(inst.primary_pane.tool, "codex");
        assert!(inst.cross_agent_team);
        assert_eq!(inst.command, "codex");
        assert!(!inst.has_command_override());
        assert!(inst.extra_args.is_empty());
        assert!(inst.resume_token.is_none());
        assert!(inst.agent_session_id.is_none());
        assert!(inst.fork_pending.is_none());
        assert_eq!(inst.xats_identity_key.as_deref(), Some("key-0"));
    }

    #[test]
    fn codex_to_claude_clears_codex_command() {
        let mut inst = claude_instance();
        inst.switch_agent("codex", &Config::default()).unwrap();
        inst.switch_agent("claude", &Config::default()).unwrap();

        assert_eq!(inst.tool, "claude");
        assert!(inst.command.is_empty());
    }

    #[test]
    fn configured_override_and_extra_args_apply_to_target() {
        let mut config = Config::default();
        config
            .session
            .agent_command_override
            .insert("codex".to_string(), "my-codex".to_string());
        config
            .session
            .agent_extra_args
            .insert("codex".to_string(), "--flag".to_string());
        let mut inst = claude_instance();
        inst.switch_agent("codex", &config).unwrap();

        assert_eq!(inst.command, "my-codex");
        assert_eq!(inst.extra_args, "--flag");
    }

    #[test]
    fn unsupported_target_is_rejected() {
        let mut inst = claude_instance();
        assert!(inst.switch_agent("shell", &Config::default()).is_err());
        assert_eq!(inst.tool, "claude");
    }

    #[test]
    fn opencode_switch_keeps_worktree_and_named_identity_but_clears_old_runtime() {
        let mut inst = claude_instance();
        let mut pane = inst.primary_pane.clone();
        pane.working_dir = "/tmp/project/.worktrees/feature".into();
        pane.worktree = Some(crate::session::PaneWorktreeInfo {
            worktree_path: Some(pane.working_dir.clone()),
            ..Default::default()
        });
        inst.set_primary_pane_config(pane.clone());
        let mut identity =
            crate::managed_xats::ManagedXats::new("worker".into(), "project".into()).unwrap();
        identity.model = Some("old-model".into());
        identity.effort = Some("high".into());
        identity.client_config = Some(serde_json::json!({"old": "runtime"}));
        inst.managed_xats = Some(identity);

        inst.switch_agent("opencode", &Config::default()).unwrap();

        assert_eq!(inst.tool, "opencode");
        assert_eq!(inst.project_path, pane.working_dir);
        assert_eq!(inst.primary_pane.worktree, pane.worktree);
        assert_eq!(inst.xats_identity_key.as_deref(), Some("key-0"));
        assert!(inst.agent_session_id.is_none());
        assert!(inst.resume_token.is_none());
        let identity = inst.managed_xats.as_ref().unwrap();
        assert_eq!((&*identity.name, &*identity.team), ("worker", "project"));
        assert!(identity.client_config.is_none());
        assert!(identity.model.is_none());
        assert!(identity.effort.is_none());
        let saved: Instance = serde_json::from_str(&serde_json::to_string(&inst).unwrap()).unwrap();
        assert_eq!(saved.tool, "opencode");
        assert_eq!(saved.primary_pane, inst.primary_pane);
    }

    #[test]
    fn managed_secondary_switch_updates_its_saved_runtime_without_changing_primary() {
        let mut inst = claude_instance();
        let config = crate::session::PaneConfig::new("claude", "/tmp/worktree", true, true);
        let identity =
            crate::managed_xats::ManagedXats::new("tester".into(), "project".into()).unwrap();
        let mut pane = crate::session::ManagedPane::new(
            uuid::Uuid::new_v4().to_string(),
            config.clone(),
            Some(identity),
        )
        .unwrap();
        pane.slot = Some(1);
        pane.native_session_id = Some("old-claude-session".into());
        let key = pane.identity_key.clone();
        inst.managed_panes.push(pane);

        inst.switch_pane_agent(1, "opencode", &Config::default())
            .unwrap();

        assert_eq!(inst.tool, "claude");
        assert_eq!(inst.agent_session_id.as_deref(), Some("sess"));
        let pane = &inst.managed_panes[0];
        assert_eq!(pane.config.tool, "opencode");
        assert_eq!(pane.config.working_dir, config.working_dir);
        assert_eq!(pane.identity_key, key);
        assert!(pane.native_session_id.is_none());
        let identity = pane.xats.as_ref().unwrap();
        assert_eq!((&*identity.name, &*identity.team), ("tester", "project"));
        let projected = pane.project(&inst).unwrap();
        assert_eq!(projected.tool, "opencode");
        assert_eq!(projected.project_path, config.working_dir);
    }
}
