//! Session instance definition and operations

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::containers::{self, ContainerRuntimeInterface, DockerContainer};
use crate::tmux;

use super::container_config;
use super::environment::{build_docker_env_args, shell_escape};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Waiting,
    #[default]
    Idle,
    Unknown,
    Stopped,
    Error,
    Starting,
    Restarting,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartPhase {
    SendingExitKeys { next_group_index: usize },
    WaitingForExit,
}

#[derive(Debug, Clone)]
pub struct PendingResume {
    pub phase: RestartPhase,
    pub config: &'static crate::agents::ResumeConfig,
    pub started_at: Instant,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct StatusUpdateOptions {
    pub allow_capture: bool,
    pub reused_status: Option<Status>,
}

impl Default for StatusUpdateOptions {
    fn default() -> Self {
        Self {
            allow_capture: true,
            reused_status: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub branch: String,
    pub main_repo_path: String,
    pub managed_by_aoe: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_true")]
    pub cleanup_on_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    pub image: String,
    pub container_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// Additional environment entries (session-specific).
    /// `KEY` = pass through from host, `KEY=VALUE` = set explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_env: Option<Vec<String>>,
    /// Custom instruction text to inject into agent launch command
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub title: String,
    pub project_path: String,
    #[serde(default)]
    pub group_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub extra_args: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub yolo_mode: bool,
    #[serde(default)]
    pub status: Status,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,

    // Git worktree integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_info: Option<WorktreeInfo>,

    // Docker sandbox integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_info: Option<SandboxInfo>,

    // Runtime state (not serialized)
    #[serde(skip)]
    pub last_error_check: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_start_time: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    #[serde(skip)]
    pub pending_resume: Option<PendingResume>,
    #[serde(skip)]
    pub last_spinner_seen: Option<Instant>,
    #[serde(skip)]
    pub spike_start: Option<Instant>,
    #[serde(skip)]
    pub pre_spike_status: Option<Status>,
    #[serde(skip)]
    pub acknowledged: bool,
}

impl Instance {
    pub fn new(title: &str, project_path: &str) -> Self {
        Self {
            id: generate_id(),
            title: title.to_string(),
            project_path: project_path.to_string(),
            group_path: String::new(),
            parent_session_id: None,
            command: String::new(),
            extra_args: String::new(),
            tool: "claude".to_string(),
            yolo_mode: false,
            status: Status::Idle,
            created_at: Utc::now(),
            last_accessed_at: None,
            worktree_info: None,
            sandbox_info: None,
            last_error_check: None,
            last_start_time: None,
            last_error: None,
            resume_token: None,
            pending_resume: None,
            last_spinner_seen: None,
            spike_start: None,
            pre_spike_status: None,
            acknowledged: false,
        }
    }

    pub fn is_sub_session(&self) -> bool {
        self.parent_session_id.is_some()
    }

    pub fn is_sandboxed(&self) -> bool {
        self.sandbox_info.as_ref().is_some_and(|s| s.enabled)
    }

    pub fn is_yolo_mode(&self) -> bool {
        self.yolo_mode
    }

    fn has_custom_command(&self) -> bool {
        if !self.extra_args.is_empty() {
            return true;
        }
        self.has_command_override()
    }

    fn has_command_override(&self) -> bool {
        if self.command.is_empty() {
            return false;
        }
        crate::agents::get_agent(&self.tool)
            .map(|a| self.command != a.binary)
            .unwrap_or(true)
    }

    pub fn can_gracefully_restart(&self) -> bool {
        !self.is_sandboxed()
            && !self.has_command_override()
            && self.pending_resume.is_none()
            && crate::agents::get_agent(&self.tool).is_some_and(|agent| agent.resume.is_some())
    }

    pub fn expects_shell(&self) -> bool {
        crate::tmux::utils::is_shell_command(self.get_tool_command())
    }

    pub fn get_tool_command(&self) -> &str {
        if self.command.is_empty() {
            crate::agents::get_agent(&self.tool)
                .map(|a| a.binary)
                .unwrap_or("bash")
        } else {
            &self.command
        }
    }

    pub fn tmux_session(&self) -> Result<tmux::Session> {
        tmux::Session::new(&self.id, &self.title)
    }

    fn sandbox_display(&self) -> Option<crate::tmux::status_bar::SandboxDisplay> {
        self.sandbox_info.as_ref().and_then(|s| {
            if s.enabled {
                Some(crate::tmux::status_bar::SandboxDisplay {
                    container_name: s.container_name.clone(),
                })
            } else {
                None
            }
        })
    }

    /// Apply all configured tmux options to a session with the given name and title.
    fn apply_session_tmux_options(&self, session_name: &str, display_title: &str) {
        let branch = self.worktree_info.as_ref().map(|w| w.branch.as_str());
        let sandbox = self.sandbox_display();
        crate::tmux::status_bar::apply_all_tmux_options(
            session_name,
            display_title,
            branch,
            sandbox.as_ref(),
        );
    }

    pub fn start(&mut self) -> Result<()> {
        self.start_with_size(None)
    }

    pub fn start_with_size(&mut self, size: Option<(u16, u16)>) -> Result<()> {
        self.start_with_size_opts(size, false)
    }

    /// Start the session, optionally skipping on_launch hooks (e.g. when they
    /// already ran in the background creation poller).
    pub fn start_with_size_opts(
        &mut self,
        size: Option<(u16, u16)>,
        skip_on_launch: bool,
    ) -> Result<()> {
        self.clear_resume_token();
        let session = self.tmux_session()?;

        if session.exists() {
            return Ok(());
        }

        // Resolve on_launch hooks from the full config chain (global > profile > repo).
        // Repo hooks go through trust verification; global/profile hooks are implicitly trusted.
        let on_launch_hooks = if skip_on_launch {
            None
        } else {
            self.resolve_on_launch_hooks()
        };

        // Install status-detection hooks for agents that support them
        let agent = crate::agents::get_agent(&self.tool);
        if let Some(hook_cfg) = agent.and_then(|a| a.hook_config.as_ref()) {
            if self.is_sandboxed() {
                // For sandboxed sessions, hooks are installed via build_container_config
            } else {
                // Install hooks in the user's home directory settings
                if let Some(home) = dirs::home_dir() {
                    let settings_path = home.join(hook_cfg.settings_rel_path);
                    if let Err(e) = crate::hooks::install_hooks(&settings_path) {
                        tracing::warn!("Failed to install agent hooks: {}", e);
                    }
                }
            }
        }

        // Ensure container is running for sandboxed sessions, then execute hooks
        if self.is_sandboxed() {
            self.get_container_for_instance()?;
            if let Some(ref hook_cmds) = on_launch_hooks {
                self.execute_on_launch_hooks(hook_cmds);
            }
        } else if let Some(ref hook_cmds) = on_launch_hooks {
            self.execute_on_launch_hooks(hook_cmds);
        }

        let cmd = self.build_agent_command(None);
        tracing::debug!("agent cmd: {}", cmd.as_ref().map_or("none", |v| v));
        session.create_with_size(&self.project_path, cmd.as_deref(), size)?;

        // Apply all configured tmux options (status bar, mouse, etc.)
        self.apply_tmux_options();

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.pending_resume = None;

        Ok(())
    }

    /// Build the agent launch command string. Pure command construction with no
    /// side effects (no hooks, no container lifecycle management).
    pub fn build_agent_command(&self, resume_token: Option<&str>) -> Option<String> {
        let agent = crate::agents::get_agent(&self.tool);

        if self.is_sandboxed() {
            let sandbox = self.sandbox_info.as_ref()?;
            let container = DockerContainer::from_session_id(&self.id);

            let base_cmd = self.build_base_tool_command(agent, resume_token);
            let mut tool_cmd = if self.is_yolo_mode() {
                if let Some(yolo) = agent.and_then(|a| a.yolo.as_ref()) {
                    match yolo {
                        crate::agents::YoloMode::CliFlag(flag) => {
                            format!("{} {}", base_cmd, flag)
                        }
                        crate::agents::YoloMode::EnvVar(..) => base_cmd,
                    }
                } else {
                    base_cmd
                }
            } else {
                base_cmd
            };
            if let Some(ref instruction) = sandbox.custom_instruction {
                if !instruction.is_empty() {
                    if let Some(flag_template) = agent.and_then(|a| a.instruction_flag) {
                        let escaped = shell_escape(instruction);
                        let flag = flag_template.replace("{}", &escaped);
                        tool_cmd = format!("{} {}", tool_cmd, flag);
                    }
                }
            }

            let mut env_args = build_docker_env_args(sandbox);
            env_args = format!("{} -e AOE_INSTANCE_ID={}", env_args, self.id);
            let env_part = format!("{} ", env_args);
            Some(wrap_command_ignore_suspend(
                &container.exec_command(Some(&env_part), &tool_cmd),
            ))
        } else {
            let needs_instance_id = agent.and_then(|a| a.hook_config.as_ref()).is_some();

            if self.command.is_empty() {
                agent.filter(|a| a.supports_host_launch).map(|a| {
                    let mut cmd = self.build_base_tool_command(Some(a), resume_token);
                    let mut env_vars: Vec<(&str, &str)> = Vec::new();
                    if needs_instance_id {
                        env_vars.push(("AOE_INSTANCE_ID", &self.id));
                    }
                    if self.is_yolo_mode() {
                        if let Some(ref yolo) = a.yolo {
                            match yolo {
                                crate::agents::YoloMode::CliFlag(flag) => {
                                    cmd = format!("{} {}", cmd, flag);
                                }
                                crate::agents::YoloMode::EnvVar(key, value) => {
                                    env_vars.push((key, value));
                                }
                            }
                        }
                    }
                    wrap_command_ignore_suspend_with_env(&cmd, &env_vars)
                })
            } else {
                let mut cmd = self.build_base_tool_command(agent, resume_token);
                let agent = crate::agents::get_agent(&self.tool);
                let mut env_vars: Vec<(&str, &str)> = Vec::new();
                if needs_instance_id {
                    env_vars.push(("AOE_INSTANCE_ID", &self.id));
                }
                if self.is_yolo_mode() {
                    if let Some(ref yolo) = agent.and_then(|a| a.yolo.as_ref()) {
                        match yolo {
                            crate::agents::YoloMode::CliFlag(flag) => {
                                cmd = format!("{} {}", cmd, flag);
                            }
                            crate::agents::YoloMode::EnvVar(key, value) => {
                                env_vars.push((key, value));
                            }
                        }
                    }
                }
                Some(wrap_command_ignore_suspend_with_env(&cmd, &env_vars))
            }
        }
    }

    fn build_base_tool_command(
        &self,
        agent: Option<&crate::agents::AgentDef>,
        resume_token: Option<&str>,
    ) -> String {
        let mut cmd = self.get_tool_command().to_string();
        if let Some(token) = resume_token {
            if let Some(resume) = agent
                .and_then(|a| a.resume.as_ref())
                .filter(|_| !self.has_command_override())
            {
                let resume_flag = resume.resume_flag.replace("{}", token);
                cmd = format!("{} {}", cmd, resume_flag);
            }
        }
        if !self.extra_args.is_empty() {
            cmd = format!("{} {}", cmd, self.extra_args);
        }
        cmd
    }

    fn resolve_on_launch_hooks(&self) -> Option<Vec<String>> {
        let profile = super::config::resolve_default_profile();
        let mut resolved = super::profile_config::resolve_config(&profile)
            .map(|c| c.hooks.on_launch)
            .unwrap_or_default();

        match super::repo_config::check_hook_trust(Path::new(&self.project_path)) {
            Ok(super::repo_config::HookTrustStatus::Trusted(hooks))
                if !hooks.on_launch.is_empty() =>
            {
                resolved = hooks.on_launch.clone();
            }
            _ => {}
        }

        if resolved.is_empty() {
            None
        } else {
            Some(resolved)
        }
    }

    fn execute_on_launch_hooks(&self, hook_cmds: &[String]) {
        if self.is_sandboxed() {
            if let Some(ref sandbox) = self.sandbox_info {
                let workdir = self.container_workdir();
                if let Err(e) = super::repo_config::execute_hooks_in_container(
                    hook_cmds,
                    &sandbox.container_name,
                    &workdir,
                ) {
                    tracing::warn!("on_launch hook failed in container: {}", e);
                }
            }
        } else if let Err(e) =
            super::repo_config::execute_hooks(hook_cmds, Path::new(&self.project_path))
        {
            tracing::warn!("on_launch hook failed: {}", e);
        }
    }

    /// Respawn only the agent pane, preserving the tmux session layout.
    /// Runs on-launch hooks, rebuilds the agent command, and respawns the pane.
    pub fn respawn_agent_pane(&mut self) -> Result<()> {
        self.respawn_agent_pane_with_resume(None)
    }

    fn respawn_agent_pane_with_resume(&mut self, resume_token: Option<&str>) -> Result<()> {
        let effective_resume_token = self.resolved_resume_token(resume_token);
        let session = self.tmux_session()?;
        if !session.exists() {
            anyhow::bail!("Session does not exist");
        }

        if let Some(ref hook_cmds) = self.resolve_on_launch_hooks() {
            self.execute_on_launch_hooks(hook_cmds);
        }

        let cmd = self
            .build_agent_command(effective_resume_token.as_deref())
            .ok_or_else(|| anyhow::anyhow!("No agent command available"))?;

        session.kill_agent_pane_process_tree();
        session.respawn_agent_pane(&cmd, &self.project_path)?;

        self.apply_tmux_options();

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.clear_resume_token();
        self.pending_resume = None;

        Ok(())
    }

    pub fn initiate_graceful_restart(&mut self) -> Result<bool> {
        if !self.can_gracefully_restart() {
            return Ok(false);
        }

        let Some(config) =
            crate::agents::get_agent(&self.tool).and_then(|agent| agent.resume.as_ref())
        else {
            return Ok(false);
        };

        let Some(first_group) = config.exit_sequence.first() else {
            return Ok(false);
        };

        let session = self.tmux_session()?;
        if !session.exists() {
            return Ok(false);
        }

        // If the agent pane is already dead, skip graceful restart. The pane
        // output is stale, so only a previously persisted token is safe to use.
        if session.is_pane_dead() {
            if let Some(token) = self.resolved_resume_token(None) {
                self.respawn_agent_pane_with_resume(Some(token.as_str()))?;
                return Ok(true);
            }
            return Ok(false);
        }

        session.send_keys_to_agent_pane(first_group)?;
        self.status = Status::Restarting;
        self.last_error = None;
        self.pending_resume = Some(PendingResume {
            phase: if config.exit_sequence.len() > 1 {
                RestartPhase::SendingExitKeys {
                    next_group_index: 1,
                }
            } else {
                RestartPhase::WaitingForExit
            },
            config,
            started_at: Instant::now(),
            timeout: Duration::from_secs(config.timeout_secs),
        });

        Ok(true)
    }

    pub fn tick_pending_resume(&mut self) -> Option<crate::tui::Action> {
        let pending = self.pending_resume.clone()?;

        match pending.phase {
            RestartPhase::SendingExitKeys { next_group_index } => {
                let session = self.tmux_session().ok()?;
                let Some(keys) = pending.config.exit_sequence.get(next_group_index) else {
                    self.mutate_pending_resume_phase(RestartPhase::WaitingForExit);
                    return None;
                };

                if let Err(err) = session.send_keys_to_agent_pane(keys) {
                    tracing::warn!("Failed to send graceful restart keys: {}", err);
                    return self.fallback_to_fresh_restart();
                }

                let next_phase = if next_group_index + 1 < pending.config.exit_sequence.len() {
                    RestartPhase::SendingExitKeys {
                        next_group_index: next_group_index + 1,
                    }
                } else {
                    RestartPhase::WaitingForExit
                };
                self.mutate_pending_resume_phase(next_phase);
                None
            }
            RestartPhase::WaitingForExit => {
                let Ok(session) = self.tmux_session() else {
                    return self.fallback_to_fresh_restart();
                };

                if session.is_pane_dead() {
                    let output = session.capture_pane_cached(100).unwrap_or_default();
                    let resume_token =
                        match extract_resume_token(&output, pending.config.resume_pattern) {
                            Some(token) if !is_valid_resume_token(&token) => {
                                tracing::warn!(
                                    "Ignoring invalid resume token extracted for '{}': {:?}",
                                    self.title,
                                    token
                                );
                                return self.fallback_to_fresh_restart();
                            }
                            token => token,
                        };
                    let action = crate::tui::Action::AttachSession(self.id.clone());
                    if let Err(err) = self.respawn_agent_pane_with_resume(resume_token.as_deref()) {
                        tracing::warn!("Failed to respawn agent pane after graceful exit: {}", err);
                        self.last_error = Some(err.to_string());
                        self.status = Status::Error;
                        self.pending_resume = None;
                        return None;
                    }
                    return Some(action);
                }

                if pending.started_at.elapsed() >= pending.timeout {
                    tracing::warn!(
                        "Graceful restart timed out for '{}' after {:?}",
                        self.title,
                        pending.timeout
                    );
                    return self.fallback_to_fresh_restart();
                }

                None
            }
        }
    }

    fn mutate_pending_resume_phase(&mut self, phase: RestartPhase) {
        if let Some(pending) = &mut self.pending_resume {
            pending.phase = phase;
        }
    }

    fn clear_resume_token(&mut self) {
        self.resume_token = None;
    }

    fn resolved_resume_token(&self, resume_token: Option<&str>) -> Option<String> {
        resume_token
            .map(std::string::ToString::to_string)
            .or_else(|| self.resume_token.clone())
    }

    fn fallback_to_fresh_restart(&mut self) -> Option<crate::tui::Action> {
        self.resume_token = None;
        let action = crate::tui::Action::AttachSession(self.id.clone());
        if let Err(err) = self.respawn_agent_pane_with_resume(None) {
            tracing::warn!("Failed to fall back to fresh restart: {}", err);
            self.last_error = Some(err.to_string());
            self.status = Status::Error;
            self.pending_resume = None;
            return None;
        }
        Some(action)
    }

    fn apply_tmux_options(&self) {
        let name = tmux::Session::generate_name(&self.id, &self.title);
        self.apply_session_tmux_options(&name, &self.title);
        if self.tool == "codex" {
            if let Err(e) = tmux::status_bar::ensure_codex_title_monitor(&name, &self.title) {
                tracing::debug!("Failed to refresh Codex title monitor: {}", e);
            }
        }
    }

    pub fn refresh_agent_tmux_options(&self) {
        self.apply_tmux_options();
    }

    pub fn get_container_for_instance(&mut self) -> Result<containers::DockerContainer> {
        let sandbox = self
            .sandbox_info
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Cannot ensure container for non-sandboxed session"))?;

        let image = &sandbox.image;
        let container = DockerContainer::new(&self.id, image);

        if container.is_running()? {
            container_config::refresh_agent_configs();
            return Ok(container);
        }

        if container.exists()? {
            container_config::refresh_agent_configs();
            container.start()?;
            return Ok(container);
        }

        // Ensure image is available (always pulls to get latest)
        let runtime = containers::get_container_runtime();
        runtime.ensure_image(image)?;

        let config = self.build_container_config()?;
        let container_id = container.create(&config)?;

        if let Some(ref mut sandbox) = self.sandbox_info {
            sandbox.container_id = Some(container_id);
            sandbox.created_at = Some(Utc::now());
        }

        Ok(container)
    }

    /// Get the container working directory for this instance.
    pub fn container_workdir(&self) -> String {
        container_config::compute_volume_paths(Path::new(&self.project_path), &self.project_path)
            .map(|(_, wd)| wd)
            .unwrap_or_else(|_| "/workspace".to_string())
    }

    fn build_container_config(&self) -> Result<crate::containers::ContainerConfig> {
        container_config::build_container_config(
            &self.project_path,
            self.sandbox_info.as_ref().unwrap(),
            &self.tool,
            self.is_yolo_mode(),
            &self.id,
        )
    }

    pub fn restart(&mut self) -> Result<()> {
        self.restart_with_size(None)
    }

    pub fn restart_with_size(&mut self, size: Option<(u16, u16)>) -> Result<()> {
        let session = self.tmux_session()?;

        if session.exists() {
            session.kill()?;
        }

        // Small delay to ensure tmux cleanup
        std::thread::sleep(std::time::Duration::from_millis(100));

        self.start_with_size(size)
    }

    pub fn kill(&self) -> Result<()> {
        let session = self.tmux_session()?;
        if session.exists() {
            session.kill()?;
        }
        Ok(())
    }

    /// Stop the session: kill the tmux session and stop the Docker container
    /// (if sandboxed). The container is stopped but not removed, so it can be
    /// restarted on re-attach.
    pub fn stop(&self) -> Result<()> {
        self.kill()?;

        if self.is_sandboxed() {
            let container = containers::DockerContainer::from_session_id(&self.id);
            if container.is_running().unwrap_or(false) {
                container.stop()?;
            }
        }

        crate::hooks::cleanup_hook_status_dir(&self.id);

        Ok(())
    }

    pub fn update_status(&mut self) {
        self.update_status_with_options(StatusUpdateOptions::default());
    }

    pub fn update_status_with_options(&mut self, options: StatusUpdateOptions) {
        if matches!(
            self.status,
            Status::Stopped | Status::Restarting | Status::Deleting
        ) {
            return;
        }

        // Skip expensive checks for recently errored sessions
        if self.status == Status::Error {
            if let Some(last_check) = self.last_error_check {
                if last_check.elapsed().as_secs() < 30 {
                    return;
                }
            }
        }

        // Grace period for starting sessions
        if let Some(start_time) = self.last_start_time {
            if start_time.elapsed().as_secs() < 3 {
                self.status = Status::Starting;
                return;
            }
        }

        let session = match self.tmux_session() {
            Ok(s) => s,
            Err(_) => {
                self.status = Status::Error;
                self.last_error_check = Some(std::time::Instant::now());
                return;
            }
        };

        if !session.exists() {
            self.status = Status::Error;
            self.last_error_check = Some(std::time::Instant::now());
            return;
        }

        let previous_status = self.status;
        let now = Instant::now();

        // Check hook-based status first (more reliable than tmux pane parsing)
        if let Some(hook_status) = crate::hooks::read_hook_status(&self.id) {
            tracing::trace!("hook status detection '{}': {:?}", self.title, hook_status);
            self.clear_spike_state();
            self.status = if session.is_pane_dead() {
                Status::Error
            } else {
                self.apply_acknowledged_mapping(hook_status)
            };
            self.last_error = None;
            return;
        }

        let session_name = tmux::Session::generate_name(&self.id, &self.title);
        if let Some(detected) = tmux::get_cached_pane_info(&session_name)
            .and_then(|info| tmux::status_detection::detect_status_from_title(&info.pane_title))
        {
            self.clear_spike_state();
            self.last_spinner_seen = Some(now);
            self.status = detected;
            self.last_error = None;
            return;
        }

        let mut detected = if options.allow_capture {
            match session.detect_status(&self.tool) {
                Ok(status) => status,
                Err(_) => Status::Idle,
            }
        } else {
            options.reused_status.unwrap_or(previous_status)
        };
        tracing::trace!(
            "status detection '{}' (tool={}, custom_cmd={}, allow_capture={}): {:?}",
            self.title,
            self.tool,
            self.has_custom_command(),
            options.allow_capture,
            detected
        );

        if options.allow_capture && detected == Status::Running {
            self.last_spinner_seen = Some(now);
        }

        if options.allow_capture {
            detected = self.apply_spike_detection(detected, previous_status, now);
            detected = self.apply_spinner_grace_period(detected, previous_status, now);
        }
        detected = self.apply_acknowledged_mapping(detected);

        // In multi-pane sessions, is_pane_running_shell may target a user-created
        // shell pane (e.g. from Ctrl+B %) rather than the agent pane when
        // @aoe_agent_pane is not set. Only treat a shell as stale for
        // single-pane sessions where a shell unambiguously means the agent exited.
        let is_single_pane = session.pane_count() <= 1;
        let is_shell_stale =
            || is_single_pane && !self.expects_shell() && session.is_pane_running_shell();
        self.status = match detected {
            Status::Idle if self.has_custom_command() => {
                if session.is_pane_dead() || is_shell_stale() {
                    Status::Error
                } else {
                    Status::Unknown
                }
            }
            Status::Idle if session.is_pane_dead() || is_shell_stale() => Status::Error,
            other => other,
        };

        // Clear stale error now that the session is healthy
        self.last_error = None;
    }

    fn clear_spike_state(&mut self) {
        self.spike_start = None;
        self.pre_spike_status = None;
    }

    fn apply_acknowledged_mapping(&self, status: Status) -> Status {
        if status == Status::Waiting && self.acknowledged {
            Status::Idle
        } else {
            status
        }
    }

    fn apply_spike_detection(
        &mut self,
        detected: Status,
        previous_status: Status,
        now: Instant,
    ) -> Status {
        if detected != Status::Running {
            self.clear_spike_state();
            return detected;
        }

        if previous_status == Status::Running {
            self.clear_spike_state();
            return Status::Running;
        }

        if self.spike_start.is_some() {
            self.clear_spike_state();
            return Status::Running;
        }

        self.spike_start = Some(now);
        self.pre_spike_status = Some(previous_status);
        previous_status
    }

    fn apply_spinner_grace_period(
        &mut self,
        detected: Status,
        previous_status: Status,
        now: Instant,
    ) -> Status {
        if previous_status == Status::Running
            && detected != Status::Running
            && self
                .last_spinner_seen
                .is_some_and(|seen| now.duration_since(seen) <= Duration::from_millis(500))
        {
            Status::Running
        } else {
            detected
        }
    }

    pub fn capture_output_with_size(
        &self,
        lines: usize,
        width: u16,
        height: u16,
    ) -> Result<String> {
        let session = self.tmux_session()?;
        session.capture_pane_with_size(lines, Some(width), Some(height))
    }
}

fn generate_id() -> String {
    Uuid::new_v4().to_string().replace("-", "")[..16].to_string()
}

/// Wrap a command to disable Ctrl-Z (SIGTSTP) suspension.
///
/// When running agents directly as tmux session commands (without a parent shell),
/// pressing Ctrl-Z suspends the process with no way to recover via job control.
/// This wrapper disables the suspend character at the terminal level before exec'ing
/// the actual command.
///
/// Uses POSIX-standard `stty susp undef` which works on both Linux and macOS.
/// Single quotes in `cmd` are escaped with the `'\''` technique to prevent
/// breaking out of the outer `bash -c '...'` wrapper.
///
/// Environment variables are exported before `exec` because `exec VAR=val cmd`
/// is not portable and fails in many shells.
fn wrap_command_ignore_suspend(cmd: &str) -> String {
    wrap_command_ignore_suspend_with_env(cmd, &[])
}

fn wrap_command_ignore_suspend_with_env(cmd: &str, env_vars: &[(&str, &str)]) -> String {
    let escaped = cmd.replace('\'', "'\\''");
    // Place env vars before `bash -c` so they're parsed at the outer shell
    // level, avoiding quoting conflicts with the inner single-quoted string.
    let env_prefix = env_vars
        .iter()
        .map(|(k, v)| {
            let escaped_v = v.replace('\'', "'\\''");
            format!("{}='{}' ", k, escaped_v)
        })
        .collect::<String>();
    format!("{}bash -c 'stty susp undef; exec {}'", env_prefix, escaped)
}

pub(crate) fn extract_resume_token(output: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(output)?
        .get(1)
        .map(|m| m.as_str().to_string())
}

pub(crate) fn is_valid_resume_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_instance() {
        let inst = Instance::new("test", "/tmp/test");
        assert_eq!(inst.title, "test");
        assert_eq!(inst.project_path, "/tmp/test");
        assert_eq!(inst.status, Status::Idle);
        assert_eq!(inst.id.len(), 16);
        assert!(inst.resume_token.is_none());
        assert!(inst.last_spinner_seen.is_none());
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
        assert!(!inst.acknowledged);
    }

    #[test]
    fn test_is_sub_session() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sub_session());

        inst.parent_session_id = Some("parent123".to_string());
        assert!(inst.is_sub_session());
    }

    #[test]
    fn test_all_agents_have_yolo_support() {
        for agent in crate::agents::AGENTS {
            if agent.name == "shell" {
                assert!(
                    agent.yolo.is_none(),
                    "Shell should not have YOLO mode configured"
                );
                continue;
            }
            assert!(
                agent.yolo.is_some(),
                "Agent '{}' should have YOLO mode configured",
                agent.name
            );
        }
    }

    #[test]
    fn test_yolo_mode_helper() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_yolo_mode());

        inst.yolo_mode = true;
        assert!(inst.is_yolo_mode());

        inst.yolo_mode = false;
        assert!(!inst.is_yolo_mode());
    }

    #[test]
    fn test_yolo_mode_without_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sandboxed());

        inst.yolo_mode = true;
        assert!(inst.is_yolo_mode());
        assert!(!inst.is_sandboxed());
    }

    // Additional tests for is_sandboxed
    #[test]
    fn test_is_sandboxed_without_sandbox_info() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(!inst.is_sandboxed());
    }

    #[test]
    fn test_is_sandboxed_with_disabled_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.sandbox_info = Some(SandboxInfo {
            enabled: false,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        assert!(!inst.is_sandboxed());
    }

    #[test]
    fn test_is_sandboxed_with_enabled_sandbox() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        assert!(inst.is_sandboxed());
    }

    // Tests for get_tool_command
    #[test]
    fn test_get_tool_command_default_claude() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        assert_eq!(inst.get_tool_command(), "claude");
    }

    #[test]
    fn test_get_tool_command_opencode() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "opencode".to_string();
        assert_eq!(inst.get_tool_command(), "opencode");
    }

    #[test]
    fn test_get_tool_command_codex() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        assert_eq!(inst.get_tool_command(), "codex");
    }

    #[test]
    fn test_get_tool_command_gemini() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "gemini".to_string();
        assert_eq!(inst.get_tool_command(), "gemini");
    }

    #[test]
    fn test_get_tool_command_unknown_tool() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "unknown".to_string();
        assert_eq!(inst.get_tool_command(), "bash");
    }

    #[test]
    fn test_get_tool_command_custom_command() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "claude --resume abc123".to_string();
        assert_eq!(inst.get_tool_command(), "claude --resume abc123");
    }

    #[test]
    fn test_wrap_command_ignore_suspend_basic() {
        assert_eq!(
            wrap_command_ignore_suspend("opencode"),
            "bash -c 'stty susp undef; exec opencode'"
        );
    }

    #[test]
    fn test_wrap_command_ignore_suspend_with_env() {
        let result = wrap_command_ignore_suspend_with_env(
            "opencode",
            &[("OPENCODE_PERMISSION", r#"{"*":"allow"}"#)],
        );
        // Env vars are placed before bash -c, not inside the single-quoted string
        assert_eq!(
            result,
            r#"OPENCODE_PERMISSION='{"*":"allow"}' bash -c 'stty susp undef; exec opencode'"#
        );
    }

    #[test]
    fn test_wrap_command_ignore_suspend_with_env_no_vars() {
        assert_eq!(
            wrap_command_ignore_suspend_with_env("claude", &[]),
            wrap_command_ignore_suspend("claude"),
        );
    }

    #[test]
    fn test_build_agent_command_inserts_claude_resume_flag_after_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_agent_command(Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"))
            .unwrap();

        assert!(
            cmd.starts_with("AOE_INSTANCE_ID='"),
            "expected hook env prefix, got {cmd}"
        );
        assert!(
            cmd.contains(
                "bash -c 'stty susp undef; exec claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11 --model sonnet --dangerously-skip-permissions'"
            ),
            "unexpected claude resume command: {cmd}"
        );
    }

    #[test]
    fn test_build_agent_command_inserts_codex_resume_flag_after_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.extra_args = "--model gpt-5".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_agent_command(Some("019d1af9-a899-7df1-8f7d-a244126e5ded"))
            .unwrap();

        assert_eq!(
            cmd,
            "bash -c 'stty susp undef; exec codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded --model gpt-5 --dangerously-bypass-approvals-and-sandbox'"
        );
    }

    #[test]
    fn test_resume_config_patterns_match_expected_agent_output() {
        let claude_resume = crate::agents::get_agent("claude")
            .and_then(|agent| agent.resume.as_ref())
            .unwrap();
        let codex_resume = crate::agents::get_agent("codex")
            .and_then(|agent| agent.resume.as_ref())
            .unwrap();

        assert_eq!(
            extract_resume_token(
                "Run claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11 to continue.",
                claude_resume.resume_pattern,
            )
            .as_deref(),
            Some("4dc7a3c8-934e-40c1-95f8-8b00fe11cf11")
        );
        assert_eq!(
            extract_resume_token(
                "Resume with: codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded",
                codex_resume.resume_pattern,
            )
            .as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_graceful_restart_skipped_for_custom_commands_and_agents_without_resume() {
        let mut custom = Instance::new("custom", "/tmp/test");
        custom.tool = "claude".to_string();
        custom.command = "wrapper".to_string();
        assert!(!custom.initiate_graceful_restart().unwrap());
        assert!(custom.pending_resume.is_none());

        let mut no_resume = Instance::new("no-resume", "/tmp/test");
        no_resume.tool = "opencode".to_string();
        assert!(!no_resume.initiate_graceful_restart().unwrap());
        assert!(no_resume.pending_resume.is_none());
    }

    // Tests for Status enum
    #[test]
    fn test_status_default() {
        let status = Status::default();
        assert_eq!(status, Status::Idle);
    }

    #[test]
    fn test_status_serialization() {
        let statuses = vec![
            Status::Running,
            Status::Waiting,
            Status::Idle,
            Status::Unknown,
            Status::Stopped,
            Status::Error,
            Status::Starting,
            Status::Restarting,
            Status::Deleting,
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: Status = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    // Tests for WorktreeInfo
    #[test]
    fn test_worktree_info_serialization() {
        let info = WorktreeInfo {
            branch: "feature/test".to_string(),
            main_repo_path: "/home/user/repo".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: WorktreeInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(info.branch, deserialized.branch);
        assert_eq!(info.main_repo_path, deserialized.main_repo_path);
        assert_eq!(info.managed_by_aoe, deserialized.managed_by_aoe);
        assert_eq!(info.cleanup_on_delete, deserialized.cleanup_on_delete);
    }

    #[test]
    fn test_worktree_info_default_cleanup_on_delete() {
        // Deserialize without cleanup_on_delete field - should default to true
        let json = r#"{"branch":"test","main_repo_path":"/path","managed_by_aoe":true,"created_at":"2024-01-01T00:00:00Z"}"#;
        let info: WorktreeInfo = serde_json::from_str(json).unwrap();
        assert!(info.cleanup_on_delete);
    }

    // Tests for SandboxInfo
    #[test]
    fn test_sandbox_info_serialization() {
        let info = SandboxInfo {
            enabled: true,
            container_id: Some("abc123".to_string()),
            image: "myimage:latest".to_string(),
            container_name: "test_container".to_string(),
            created_at: Some(Utc::now()),
            extra_env: Some(vec!["MY_VAR".to_string(), "OTHER_VAR".to_string()]),
            custom_instruction: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: SandboxInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(info.enabled, deserialized.enabled);
        assert_eq!(info.container_id, deserialized.container_id);
        assert_eq!(info.image, deserialized.image);
        assert_eq!(info.container_name, deserialized.container_name);
        assert_eq!(info.extra_env, deserialized.extra_env);
    }

    #[test]
    fn test_sandbox_info_minimal_serialization() {
        // Required fields: enabled, image, container_name
        let json = r#"{"enabled":false,"image":"test-image","container_name":"test"}"#;
        let info: SandboxInfo = serde_json::from_str(json).unwrap();

        assert!(!info.enabled);
        assert_eq!(info.image, "test-image");
        assert_eq!(info.container_name, "test");
        assert!(info.container_id.is_none());
        assert!(info.created_at.is_none());
    }

    // Tests for Instance serialization
    #[test]
    fn test_instance_serialization_roundtrip() {
        let mut inst = Instance::new("Test Project", "/home/user/project");
        inst.tool = "claude".to_string();
        inst.group_path = "work/clients".to_string();
        inst.command = "claude --resume xyz".to_string();

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert_eq!(inst.id, deserialized.id);
        assert_eq!(inst.title, deserialized.title);
        assert_eq!(inst.project_path, deserialized.project_path);
        assert_eq!(inst.group_path, deserialized.group_path);
        assert_eq!(inst.tool, deserialized.tool);
        assert_eq!(inst.command, deserialized.command);
    }

    #[test]
    fn test_instance_deserialization_defaults_resume_token_to_none() {
        let json = r#"{
            "id":"deadbeefcafebabe",
            "title":"Test Project",
            "project_path":"/tmp/test-project",
            "status":"idle",
            "created_at":"2024-01-01T00:00:00Z"
        }"#;

        let deserialized: Instance = serde_json::from_str(json).unwrap();

        assert!(deserialized.resume_token.is_none());
    }

    #[test]
    fn test_instance_resume_token_roundtrip() {
        let mut inst = Instance::new("Test Project", "/home/user/project");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.resume_token.as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_instance_serialization_skips_runtime_fields() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.last_error_check = Some(std::time::Instant::now());
        inst.last_start_time = Some(std::time::Instant::now());
        inst.last_error = Some("test error".to_string());

        let json = serde_json::to_string(&inst).unwrap();

        // Runtime fields should not appear in JSON
        assert!(!json.contains("last_error_check"));
        assert!(!json.contains("last_start_time"));
        assert!(!json.contains("last_error"));
    }

    #[test]
    fn test_instance_with_worktree_info() {
        let mut inst = Instance::new("Test", "/tmp/worktree");
        inst.worktree_info = Some(WorktreeInfo {
            branch: "feature/abc".to_string(),
            main_repo_path: "/tmp/main".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        });

        let json = serde_json::to_string(&inst).unwrap();
        let deserialized: Instance = serde_json::from_str(&json).unwrap();

        assert!(deserialized.worktree_info.is_some());
        let wt = deserialized.worktree_info.unwrap();
        assert_eq!(wt.branch, "feature/abc");
        assert!(wt.managed_by_aoe);
    }

    // Test generate_id function properties
    #[test]
    fn test_generate_id_uniqueness() {
        let ids: Vec<String> = (0..100).map(|_| Instance::new("t", "/t").id).collect();
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len());
    }

    #[test]
    fn test_generate_id_format() {
        let inst = Instance::new("test", "/tmp/test");
        // ID should be 16 hex characters
        assert_eq!(inst.id.len(), 16);
        assert!(inst.id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_has_custom_command_empty() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(!inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_treats_extra_args_as_custom() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_same_as_agent_binary() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "claude".to_string();
        assert!(!inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.command = "my-wrapper".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_has_custom_command_unknown_tool() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "unknown_agent".to_string();
        inst.command = "some-binary".to_string();
        assert!(inst.has_custom_command());
    }

    #[test]
    fn test_can_gracefully_restart_allows_extra_args_without_command_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        assert!(inst.can_gracefully_restart());
    }

    #[test]
    fn test_is_valid_resume_token_accepts_hex_and_hyphen() {
        assert!(is_valid_resume_token(
            "019d1af9-a899-7df1-8f7d-a244126e5ded"
        ));
        assert!(is_valid_resume_token(
            "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"
        ));
    }

    #[test]
    fn test_is_valid_resume_token_rejects_invalid_characters() {
        assert!(!is_valid_resume_token(""));
        assert!(!is_valid_resume_token("abc def"));
        assert!(!is_valid_resume_token("abc$def"));
        assert!(!is_valid_resume_token("resume-token"));
    }

    #[test]
    fn test_expects_shell() {
        let mut inst = Instance::new("test", "/tmp/test");
        assert!(!inst.expects_shell());

        inst.tool = "unknown-tool".to_string();
        inst.command = String::new();
        assert!(inst.expects_shell());

        inst.tool = "claude".to_string();
        inst.command = "bash".to_string();
        assert!(inst.expects_shell());

        inst.command = "my-agent".to_string();
        assert!(!inst.expects_shell());
    }

    #[test]
    fn test_status_unknown_serialization() {
        let status = Status::Unknown;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"unknown\"");
        let deserialized: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, Status::Unknown);
    }

    #[test]
    fn test_pending_resume_is_runtime_only() {
        let mut inst = Instance::new("Test", "/tmp/test");
        let resume = crate::agents::get_agent("claude")
            .and_then(|agent| agent.resume.as_ref())
            .unwrap();
        inst.pending_resume = Some(PendingResume {
            phase: RestartPhase::WaitingForExit,
            config: resume,
            started_at: Instant::now(),
            timeout: Duration::from_secs(10),
        });

        let json = serde_json::to_string(&inst).unwrap();
        assert!(!json.contains("pending_resume"));
    }

    #[test]
    fn test_clear_resume_token_helper_clears_stored_token() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        inst.clear_resume_token();

        assert!(inst.resume_token.is_none());
    }

    #[test]
    fn test_resolved_resume_token_uses_stored_token_when_explicit_token_missing() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        assert_eq!(
            inst.resolved_resume_token(None).as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_resolved_resume_token_prefers_explicit_token() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.resume_token = Some("stored-token".to_string());

        assert_eq!(
            inst.resolved_resume_token(Some("019d1af9-a899-7df1-8f7d-a244126e5ded"))
                .as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_dead_pane_restart_would_use_stored_resume_token_when_available() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.resume_token = Some("019d1af9-a899-7df1-8f7d-a244126e5ded".to_string());

        // The tmux-backed dead-pane restart is covered at runtime. This unit
        // test covers the stored-token selection used by that branch.
        assert!(inst.can_gracefully_restart());
        assert_eq!(
            inst.resolved_resume_token(None).as_deref(),
            Some("019d1af9-a899-7df1-8f7d-a244126e5ded")
        );
    }

    #[test]
    fn test_acknowledged_waiting_maps_to_idle() {
        let mut inst = Instance::new("test", "/tmp/test");

        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Waiting),
            Status::Waiting
        );

        inst.acknowledged = true;
        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Waiting),
            Status::Idle
        );
        assert_eq!(
            inst.apply_acknowledged_mapping(Status::Running),
            Status::Running
        );
    }

    #[test]
    fn test_spinner_grace_period_holds_running() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();
        inst.last_spinner_seen = Some(now);

        assert_eq!(
            inst.apply_spinner_grace_period(
                Status::Idle,
                Status::Running,
                now + Duration::from_millis(400)
            ),
            Status::Running
        );
        assert_eq!(
            inst.apply_spinner_grace_period(
                Status::Idle,
                Status::Running,
                now + Duration::from_millis(600)
            ),
            Status::Idle
        );
    }

    #[test]
    fn test_spike_detection_requires_confirmation() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();

        let first = inst.apply_spike_detection(Status::Running, Status::Idle, now);
        assert_eq!(first, Status::Idle);
        assert!(inst.spike_start.is_some());
        assert_eq!(inst.pre_spike_status, Some(Status::Idle));

        let second = inst.apply_spike_detection(
            Status::Running,
            Status::Idle,
            now + Duration::from_millis(500),
        );
        assert_eq!(second, Status::Running);
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
    }

    #[test]
    fn test_spike_detection_rejects_transient_running() {
        let mut inst = Instance::new("test", "/tmp/test");
        let now = Instant::now();

        let first = inst.apply_spike_detection(Status::Running, Status::Waiting, now);
        assert_eq!(first, Status::Waiting);
        assert!(inst.spike_start.is_some());

        let second = inst.apply_spike_detection(
            Status::Idle,
            Status::Waiting,
            now + Duration::from_millis(500),
        );
        assert_eq!(second, Status::Idle);
        assert!(inst.spike_start.is_none());
        assert!(inst.pre_spike_status.is_none());
    }
}
