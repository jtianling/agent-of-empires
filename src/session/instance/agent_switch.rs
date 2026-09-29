use anyhow::{bail, Result};

use super::Instance;
use crate::session::Config;

impl Instance {
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
        assert!(inst.switch_agent("opencode", &Config::default()).is_err());
        assert_eq!(inst.tool, "claude");
    }
}
