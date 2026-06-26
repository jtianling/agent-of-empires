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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalInfo {
    #[serde(default)]
    pub created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
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

/// Screen markers that identify Claude's startup confirmation prompts. Each
/// default-highlights the safe-to-proceed option, so a single Enter confirms.
const AUTO_CONFIRM_MARKERS: &[&str] = &[
    "Loading development channels",
    "I am using this for local development",
    "Quick safety check",
    "trust this folder",
];

/// Max time to wait for Claude's confirmation screens before giving up and
/// attaching anyway (claude shows the dev-channels gate within ~1-2s).
const AUTO_CONFIRM_TIMEOUT: Duration = Duration::from_secs(12);
/// Delay after sending Enter before polling again, so the next screen can render.
const AUTO_CONFIRM_SEND_INTERVAL: Duration = Duration::from_millis(600);
/// Poll cadence while waiting for a confirmation screen to appear.
const AUTO_CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Once at least one Enter has been sent, stop after this long with no marker
/// on screen (Claude has moved past the confirmation prompts).
const AUTO_CONFIRM_DONE_GRACE: Duration = Duration::from_millis(1200);

/// Strip ANSI/CSI escape sequences (e.g. SGR color codes) from captured pane
/// content. Claude colors the warning title per-word, so `tmux capture-pane -e`
/// interleaves escape codes between words ("Loading\x1b[0m development...");
/// stripping them restores contiguous text for substring matching.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI sequence: ESC '[' params... final-byte (e.g. 'm'). Drop through
            // the final alphabetic byte. Other escapes: just drop the ESC.
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whether the captured pane content shows one of Claude's confirmation screens.
/// Strips ANSI escapes first so per-word coloring does not break matching.
fn is_auto_confirm_screen(output: &str) -> bool {
    let plain = strip_ansi(output);
    AUTO_CONFIRM_MARKERS.iter().any(|m| plain.contains(m))
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
    #[serde(default)]
    pub cleanup_on_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRepo {
    pub name: String,
    pub source_path: String,
    pub branch: String,
    pub worktree_path: String,
    pub main_repo_path: String,
    pub managed_by_aoe: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub branch: String,
    pub workspace_dir: String,
    pub repos: Vec<WorkspaceRepo>,
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
    /// When set (claude, non-sandboxed only), launches with the xats
    /// development-channels flag and auto-confirms Claude's startup screens.
    #[serde(default)]
    pub cross_agent_team: bool,
    /// Development-channels string appended after
    /// `--dangerously-load-development-channels` when `cross_agent_team` is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cross_agent_team_channel: String,
    #[serde(default)]
    pub status: Status,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,

    // Git worktree integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_info: Option<WorktreeInfo>,

    // Multi-repo workspace integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_info: Option<WorkspaceInfo>,

    // Docker sandbox integration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_info: Option<SandboxInfo>,

    // Paired terminal session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_info: Option<TerminalInfo>,

    /// Runtime-only: which profile this instance was loaded from. Not persisted to disk.
    #[serde(default, skip_serializing)]
    pub source_profile: String,

    // Runtime state (not serialized)
    #[serde(skip)]
    pub last_error_check: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_start_time: Option<std::time::Instant>,
    #[serde(skip)]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    /// When set, indicates this session is a pending fork of another session.
    /// The stored value is the parent agent's session token (e.g. Claude/Codex UUID
    /// or OpenCode `ses_...` id). On first successful launch, `build_base_pane_command`
    /// uses the agent's `fork_template` with this value, and the field is then cleared
    /// so subsequent restarts follow the normal resume path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_pending: Option<String>,
    /// Pre-allocated agent session UUID. When AoE starts a Claude session it
    /// passes `--session-id <uuid>` so we always know which conversation
    /// belongs to this instance. Used as the primary source for `fork_token()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    /// Runtime-only flag: set while a multi-pane `R` restart is in flight so a
    /// second `R` press on the same instance is ignored. Cleared once every
    /// tracked pane has been respawned. Never persisted.
    #[serde(skip)]
    pub restart_in_flight: bool,
    #[serde(skip)]
    pub last_spinner_seen: Option<Instant>,
    #[serde(skip)]
    pub spike_start: Option<Instant>,
    #[serde(skip)]
    pub pre_spike_status: Option<Status>,
    #[serde(skip)]
    pub acknowledged: bool,
    /// Runtime-only: when `tool == "shell"` and the user detaches from the
    /// session with an agent (claude/codex/gemini/...) running in the primary
    /// pane, this field caches the detected agent name. The status poller
    /// uses it to dispatch to that agent's content detector instead of the
    /// shell stub. Cleared to `None` when detection returns `shell` or
    /// unknown. Never persisted to disk: on aoe restart every session starts
    /// with `None` and the next detach cycle repopulates it.
    #[serde(skip, default)]
    pub detected_inner_agent: Option<String>,
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
            cross_agent_team: false,
            cross_agent_team_channel: String::new(),
            status: Status::Idle,
            created_at: Utc::now(),
            last_accessed_at: None,
            worktree_info: None,
            workspace_info: None,
            sandbox_info: None,
            terminal_info: None,
            source_profile: String::new(),
            last_error_check: None,
            last_start_time: None,
            last_error: None,
            resume_token: None,
            fork_pending: None,
            agent_session_id: None,
            restart_in_flight: false,
            last_spinner_seen: None,
            spike_start: None,
            pre_spike_status: None,
            acknowledged: false,
            detected_inner_agent: None,
        }
    }

    fn current_profile() -> String {
        std::env::var("AGENT_OF_EMPIRES_PROFILE")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| super::DEFAULT_PROFILE.to_string())
    }

    pub fn is_sub_session(&self) -> bool {
        self.parent_session_id.is_some()
    }

    pub fn is_workspace(&self) -> bool {
        self.workspace_info.is_some()
    }

    pub fn is_sandboxed(&self) -> bool {
        self.sandbox_info.as_ref().is_some_and(|s| s.enabled)
    }

    pub fn is_yolo_mode(&self) -> bool {
        self.yolo_mode
    }

    /// Whether this instance launches in Cross Agent Team mode (claude only).
    pub fn is_cross_agent_team(&self) -> bool {
        self.cross_agent_team && self.tool == "claude"
    }

    /// Auto-confirm Claude's startup screens (dev-channels warning and the
    /// workspace-trust prompt) by polling the agent pane and sending Enter while
    /// a recognized confirmation marker is shown. Runs SYNCHRONOUSLY before the
    /// caller attaches: at this point the pane exists and Claude renders into the
    /// tmux virtual terminal even with no client attached, and there is no
    /// concurrent `tmux attach` to contend with the capture/send subprocesses
    /// (a background thread would stall once attach starts). No-ops when the
    /// session is not in Cross Agent Team mode.
    fn run_auto_confirm(&self) {
        if !self.is_cross_agent_team() {
            return;
        }
        let Ok(session) = self.tmux_session() else {
            return;
        };
        let start = Instant::now();
        let mut sent = 0u32;
        let mut last_marker_seen = Instant::now();
        while start.elapsed() < AUTO_CONFIRM_TIMEOUT {
            let output = session.capture_pane(80).unwrap_or_default();
            if is_auto_confirm_screen(&output) {
                last_marker_seen = Instant::now();
                if let Err(err) = session.send_keys_to_agent_pane(&["Enter"]) {
                    tracing::warn!("auto-confirm send failed: {}", err);
                    return;
                }
                sent += 1;
                std::thread::sleep(AUTO_CONFIRM_SEND_INTERVAL);
            } else {
                if sent >= 1 && last_marker_seen.elapsed() >= AUTO_CONFIRM_DONE_GRACE {
                    return;
                }
                std::thread::sleep(AUTO_CONFIRM_POLL_INTERVAL);
            }
        }
    }

    /// The `--dangerously-load-development-channels <channel>` flag for Cross
    /// Agent Team launches, or `None` when the mode is off. Falls back to the
    /// default channel when the stored channel is empty.
    fn cross_agent_team_flag(&self) -> Option<String> {
        if !self.is_cross_agent_team() {
            return None;
        }
        let channel = if self.cross_agent_team_channel.is_empty() {
            "server:cross-agent-teams-channel"
        } else {
            self.cross_agent_team_channel.as_str()
        };
        Some(format!(
            "--dangerously-load-development-channels {}",
            channel
        ))
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
    fn apply_session_tmux_options(&self, session_name: &str, display_title: &str, profile: &str) {
        let branch = self
            .worktree_info
            .as_ref()
            .map(|w| w.branch.as_str())
            .or_else(|| self.workspace_info.as_ref().map(|w| w.branch.as_str()));
        let sandbox = self.sandbox_display();
        crate::tmux::status_bar::apply_all_tmux_options(
            session_name,
            display_title,
            branch,
            sandbox.as_ref(),
            profile,
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
                    if let Err(e) =
                        crate::hooks::install_hooks(&settings_path, hook_cfg.events, &self.tool)
                    {
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

        // Pre-allocate an agent session UUID for tools that support
        // `--session-id`. This lets AoE know the conversation identity
        // from the start (needed for fork, and avoids relying on
        // post-hoc pane scraping or disk scanning).
        if self.agent_session_id.is_none()
            && self.resume_token.is_none()
            && self.fork_pending.is_none()
            && !self.has_command_override()
        {
            if let Some(agent) = crate::agents::get_agent(&self.tool) {
                if agent.session_id_flag.is_some() {
                    self.agent_session_id = Some(Uuid::new_v4().to_string());
                }
            }
        }

        let cmd = self.build_agent_command(None);
        tracing::debug!("agent cmd: {}", cmd.as_ref().map_or("none", |v| v));
        session.create_with_size(&self.project_path, cmd.as_deref(), size)?;

        self.run_auto_confirm();

        // Apply all configured tmux options (status bar, mouse, etc.)
        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.restart_in_flight = false;
        // First launch of a forked session has been committed to tmux. The
        // agent will now spawn its own session id, so we no longer need the
        // parent's token and subsequent restarts follow the normal resume flow.
        self.fork_pending = None;

        Ok(())
    }

    /// Build the agent launch command string. Pure command construction with no
    /// side effects (no hooks, no container lifecycle management).
    ///
    /// Delegates to [`build_pane_command`](Self::build_pane_command) for the
    /// primary agent (`self.tool`, `is_primary = true`) so that the single-pane
    /// start/respawn path and the slot-based multi-pane resume path share one
    /// launch-context decoration pipeline.
    pub fn build_agent_command(&self, resume_token: Option<&str>) -> Option<String> {
        let tool = self.tool.clone();
        self.build_pane_command(&tool, resume_token, true)
    }

    /// Build the launch command for a single pane, applying the full launch
    /// context (resume flag, YOLO mode, cross-agent-team flag, `AOE_INSTANCE_ID`
    /// for hook-config agents, sandbox `docker exec` wrapping, custom instruction,
    /// and command override). This is the one decoration pipeline shared by the
    /// single-pane start/respawn path and the slot-based multi-pane resume path.
    ///
    /// `target_agent` is the agent that runs in this pane (the instance tool for
    /// the primary pane, or a slot's recorded agent for a secondary pane).
    /// `is_primary` is true only for the instance's primary pane (slot 0): the
    /// command override (`self.command`), pre-allocated session id, fork token,
    /// and `extra_args` are instance-primary concepts and apply to that pane only.
    /// Secondary panes build from their own agent binary.
    pub fn build_pane_command(
        &self,
        target_agent: &str,
        resume_token: Option<&str>,
        is_primary: bool,
    ) -> Option<String> {
        let agent = crate::agents::get_agent(target_agent);

        if self.is_sandboxed() {
            let sandbox = self.sandbox_info.as_ref()?;
            let container = DockerContainer::from_session_id(&self.id);

            let base_cmd = self.build_base_pane_command(agent, resume_token, is_primary);
            let mut tool_cmd = if self.is_yolo_mode() {
                if let Some(yolo) = agent.and_then(|a| a.yolo.as_ref()) {
                    match yolo {
                        crate::agents::YoloMode::CliFlag(flag) => {
                            format!("{} {}", base_cmd, flag)
                        }
                        crate::agents::YoloMode::EnvVar(..)
                        | crate::agents::YoloMode::AlwaysYolo => base_cmd,
                    }
                } else {
                    base_cmd
                }
            } else {
                base_cmd
            };
            if is_primary {
                if let Some(ref instruction) = sandbox.custom_instruction {
                    if !instruction.is_empty() {
                        if let Some(flag_template) = agent.and_then(|a| a.instruction_flag) {
                            let escaped = shell_escape(instruction);
                            let flag = flag_template.replace("{}", &escaped);
                            tool_cmd = format!("{} {}", tool_cmd, flag);
                        }
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
            let has_override = is_primary && !self.command.is_empty();

            if !has_override {
                agent.filter(|a| a.supports_host_launch).map(|a| {
                    let mut cmd = self.build_base_pane_command(Some(a), resume_token, is_primary);
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
                                crate::agents::YoloMode::AlwaysYolo => {}
                            }
                        }
                    }
                    if let Some(flag) = self.cross_agent_team_flag() {
                        cmd = format!("{} {}", cmd, flag);
                    }
                    wrap_command_ignore_suspend_with_env(&cmd, &env_vars)
                })
            } else {
                let mut cmd = self.build_base_pane_command(agent, resume_token, is_primary);
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
                            crate::agents::YoloMode::AlwaysYolo => {}
                        }
                    }
                }
                if let Some(flag) = self.cross_agent_team_flag() {
                    cmd = format!("{} {}", cmd, flag);
                }
                if self.expects_shell() && env_vars.is_empty() {
                    let escaped_dir = shell_escape(&self.project_path);
                    let shell = crate::session::environment::user_posix_shell();
                    let inner = format!("cd {escaped_dir} && stty susp undef; exec {cmd}");
                    let escaped_inner = inner.replace('\'', "'\\''");
                    return Some(format!("{shell} -lc '{escaped_inner}'"));
                }
                Some(wrap_command_ignore_suspend_with_env(&cmd, &env_vars))
            }
        }
    }

    /// Build the bare tool command (binary + resume/fork/session-id flags +
    /// extra args) for a single pane before launch-context decoration.
    ///
    /// For the primary pane (`is_primary = true`) this honors the instance
    /// command override (`self.command`), `extra_args`, pre-allocated session id,
    /// and fork token, matching the single-pane start/respawn path byte-for-byte.
    /// For secondary panes (`is_primary = false`) those instance-primary concepts
    /// do not apply: the command is built from the slot agent's own binary plus,
    /// when present, a resume flag from the supplied token.
    fn build_base_pane_command(
        &self,
        agent: Option<&crate::agents::AgentDef>,
        resume_token: Option<&str>,
        is_primary: bool,
    ) -> String {
        if !is_primary {
            let mut cmd = agent.map_or_else(|| "bash".to_string(), |a| a.binary.to_string());
            if let (Some(token), Some(resume)) =
                (resume_token, agent.and_then(|a| a.resume.as_ref()))
            {
                let resume_flag = resume.resume_flag.replace("{}", token);
                cmd = format!("{} {}", cmd, resume_flag);
            }
            return cmd;
        }

        let mut cmd = self.get_tool_command().to_string();
        if let Some(token) = resume_token {
            // A live resume token always wins: once the forked session has spawned
            // and AoE has captured its own post-fork session id, subsequent restarts
            // go through the normal resume path.
            if let Some(resume) = agent
                .and_then(|a| a.resume.as_ref())
                .filter(|_| !self.has_command_override())
            {
                let resume_flag = resume.resume_flag.replace("{}", token);
                cmd = format!("{} {}", cmd, resume_flag);
            }
        } else if let Some(fork_token) = self.fork_pending.as_deref() {
            // First launch of a forked session: use the agent's native fork command
            // with the parent's session token as the source. For Claude we also
            // pre-allocate a new session-id for the fork (like agent-deck does).
            if let Some(template) = agent
                .and_then(|a| a.fork_template)
                .filter(|_| !self.has_command_override())
            {
                let fork_flag = template.replace("{}", fork_token);
                cmd = format!("{} {}", cmd, fork_flag);
            }
            if let (Some(new_id), Some(flag)) = (
                self.agent_session_id.as_deref(),
                agent.and_then(|a| a.session_id_flag),
            ) {
                let id_flag = flag.replace("{}", new_id);
                cmd = format!("{} {}", cmd, id_flag);
            }
        } else if let Some(session_id) = self.agent_session_id.as_deref() {
            // Fresh launch with pre-allocated session identity.
            if let Some(flag) = agent
                .and_then(|a| a.session_id_flag)
                .filter(|_| !self.has_command_override())
            {
                let id_flag = flag.replace("{}", session_id);
                cmd = format!("{} {}", cmd, id_flag);
            }
        }
        if !self.extra_args.is_empty() {
            cmd = format!("{} {}", cmd, self.extra_args);
        }
        cmd
    }

    /// Build a new Instance that will, on its first launch, execute this agent's
    /// native fork-session command against `self` as the parent. Runtime state
    /// (status, timestamps, resume token) is cleared; persistent configuration
    /// (tool, group, worktree, sandbox) is inherited.
    ///
    /// The forked session reuses the parent's `project_path` so both the agent
    /// pane and the optional right shell pane land in the same working directory.
    /// Inherited worktree metadata is marked `cleanup_on_delete = false` so
    /// deleting the fork never destroys the parent's worktree.
    pub fn create_fork(&self, new_title: String, new_group: Option<String>) -> Result<Self> {
        let fork_token = self.fork_token()?;

        let mut fork = self.clone();
        fork.id = generate_id();
        fork.title = new_title;
        fork.parent_session_id = Some(self.id.clone());
        fork.fork_pending = Some(fork_token);

        if let Some(group) = new_group {
            fork.group_path = group;
        }

        // Clear runtime state — the fork is a fresh process lifecycle.
        fork.status = Status::Idle;
        fork.created_at = Utc::now();
        fork.last_accessed_at = None;
        fork.resume_token = None;
        fork.restart_in_flight = false;
        // Pre-allocate a new session UUID for the fork if the tool supports it.
        // This is passed via `--session-id <uuid>` alongside the fork template.
        fork.agent_session_id = crate::agents::get_agent(&self.tool)
            .filter(|a| a.session_id_flag.is_some())
            .map(|_| Uuid::new_v4().to_string());
        fork.last_error = None;
        fork.last_error_check = None;
        fork.last_start_time = None;
        fork.last_spinner_seen = None;
        fork.spike_start = None;
        fork.pre_spike_status = None;
        fork.acknowledged = false;
        fork.terminal_info = None;

        // Inherit worktree without taking ownership of cleanup: the parent
        // still relies on it.
        if let Some(ref mut wt) = fork.worktree_info {
            wt.cleanup_on_delete = false;
        }
        if let Some(ref mut ws) = fork.workspace_info {
            ws.cleanup_on_delete = false;
        }

        // Give the fork its own container name derived from the new id so it
        // does not collide with the parent's container.
        if let Some(ref mut sandbox) = fork.sandbox_info {
            sandbox.container_name = DockerContainer::generate_name(&fork.id);
            sandbox.container_id = None;
            sandbox.created_at = None;
        }

        Ok(fork)
    }

    /// Resolve the parent agent's session token to be used for forking.
    /// Returns an error for tools that do not support forking, for instances
    /// with a user-supplied command override, or for forkable tools that have
    /// not yet produced a session id AoE can capture.
    fn fork_token(&self) -> Result<String> {
        let agent = crate::agents::get_agent(&self.tool)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", self.tool))?;
        if agent.fork_template.is_none() {
            anyhow::bail!(
                "Fork is not supported for agent '{}'. Supported: claude, codex, opencode.",
                self.tool
            );
        }
        if self.has_command_override() {
            anyhow::bail!(
                "Cannot fork a session with a custom command override (command = {:?})",
                self.command
            );
        }

        match self.tool.as_str() {
            "claude" => self
                .agent_session_id
                .clone()
                .or_else(|| self.resume_token.clone())
                .or_else(|| crate::hooks::read_hook_session_id(&self.id))
                .or_else(|| resolve_claude_session_from_disk(&self.project_path))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No active Claude session found. Start and interact with the parent session, then try again."
                    )
                }),
            "codex" => self
                .agent_session_id
                .clone()
                .or_else(|| self.resume_token.clone())
                .or_else(|| crate::hooks::read_hook_session_id(&self.id))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No active codex session to fork yet. Press 'r' (restart) on the parent to capture a resume token, then try again."
                    )
                }),
            "opencode" => self.resolve_opencode_session_id(),
            other => anyhow::bail!("Fork is not supported for agent '{}'", other),
        }
    }

    /// Look up the parent opencode session id by querying
    /// `opencode session list --format json` (either on the host or inside the
    /// parent's container, depending on whether the parent is sandboxed), then
    /// picking the most recently updated session whose directory matches
    /// `self.project_path`.
    ///
    /// `resolve_claude_session_from_disk` is a companion free function for Claude.
    fn resolve_opencode_session_id(&self) -> Result<String> {
        use std::process::Command;

        let output = if self.is_sandboxed() {
            let container = DockerContainer::from_session_id(&self.id);
            if !container.is_running().unwrap_or(false) {
                anyhow::bail!(
                    "Cannot fork opencode session: parent container '{}' is not running. \
                     Start the parent session before forking.",
                    container.name
                );
            }
            Command::new("docker")
                .args([
                    "exec",
                    &container.name,
                    "opencode",
                    "session",
                    "list",
                    "--format",
                    "json",
                ])
                .output()
        } else {
            Command::new("opencode")
                .args(["session", "list", "--format", "json"])
                .current_dir(&self.project_path)
                .output()
        };

        let output =
            output.map_err(|e| anyhow::anyhow!("Failed to run `opencode session list`: {}", e))?;
        if !output.status.success() {
            anyhow::bail!(
                "`opencode session list` exited non-zero: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        #[derive(Deserialize)]
        struct OpenCodeSession {
            id: String,
            #[serde(default)]
            directory: Option<String>,
            #[serde(default)]
            path: Option<String>,
            #[serde(default)]
            updated: Option<i64>,
            #[serde(default)]
            created: Option<i64>,
        }

        let sessions: Vec<OpenCodeSession> = serde_json::from_slice(&output.stdout)
            .map_err(|e| anyhow::anyhow!("Failed to parse opencode session list: {}", e))?;

        let target = match std::fs::canonicalize(&self.project_path) {
            Ok(p) => p,
            Err(_) => std::path::PathBuf::from(&self.project_path),
        };

        let best = sessions
            .into_iter()
            .filter_map(|s| {
                let dir = s.directory.clone().or_else(|| s.path.clone())?;
                let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone().into());
                if canonical == target {
                    let ts = s.updated.or(s.created).unwrap_or(0);
                    Some((ts, s.id))
                } else {
                    None
                }
            })
            .max_by_key(|(ts, _)| *ts)
            .map(|(_, id)| id);

        best.ok_or_else(|| {
            anyhow::anyhow!(
                "No opencode session found for directory {}. Start or interact with opencode in this directory before forking.",
                self.project_path
            )
        })
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

        self.run_auto_confirm();

        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());
        self.clear_resume_token();

        Ok(())
    }

    /// Resume every tracked agent pane of this instance from the persisted
    /// `agent_slot` store. Each pane is killed and respawned with the resume
    /// command built from its own `native_session_id`; a pane that cannot resume
    /// degrades to a fresh restart of that pane only and does not abort the
    /// remaining panes. Returns the per-pane outcomes (one per slot). When the
    /// instance has no tracked slots the caller falls back to the single-pane
    /// `respawn_agent_pane` behavior.
    pub fn resume_all_tracked_panes(
        &mut self,
        slots: &[crate::db::AgentSlot],
    ) -> Vec<PaneResumeOutcome> {
        self.status = Status::Restarting;
        self.last_error = None;

        if let Some(ref hook_cmds) = self.resolve_on_launch_hooks() {
            self.execute_on_launch_hooks(hook_cmds);
        }

        let mut outcomes = Vec::with_capacity(slots.len());
        for slot in slots {
            let outcome = self.resume_launch_pane(
                &slot.agent,
                &slot.native_session_id,
                &slot.tmux_pane,
                &slot.cwd,
                slot.slot == 0,
            );
            if let PaneResumeOutcome::Error(ref err) = outcome {
                tracing::warn!(
                    "Failed to resume pane {} (slot {}) for '{}': {}",
                    slot.tmux_pane,
                    slot.slot,
                    self.title,
                    err
                );
            }
            outcomes.push(outcome);
        }

        self.run_auto_confirm();
        self.apply_tmux_options(&Self::current_profile());

        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());

        outcomes
    }

    /// Whether this instance can be cold-start recovered: it has persisted
    /// `agent_slot` rows but its tmux session no longer exists. Slot presence is
    /// supplied by the caller (read from the store) so detection stays a pure
    /// function of `has_slots` and live tmux state.
    pub fn is_recoverable(&self, has_slots: bool) -> bool {
        is_recoverable_from(
            has_slots,
            self.tmux_session().map(|s| s.exists()).unwrap_or(false),
        )
    }

    /// Rebuild this instance's tmux session from its persisted slots and resume
    /// each pane from its `native_session_id`. The session is recreated through
    /// the normal start path so worktree/sandbox context is restored, then one
    /// pane per slot is created in ascending slot order (slot 0 is the primary
    /// `@aoe_agent_pane`, the rest are split off), each pane is resume-launched
    /// via [`resume_launch_pane`], the new pane ids are written back into
    /// `agent_slot.tmux_pane`, and `@aoe_agent_pane` is re-pinned to slot 0.
    ///
    /// Per-pane failures are collected into the returned outcomes and never abort
    /// recovery of sibling panes. Returns an error only when the session/pane
    /// rebuild itself fails (before any per-pane resume runs) or when the created
    /// pane count does not match the slot count.
    pub fn recover_from_slots(
        &mut self,
        store: &crate::db::Store,
        slots: &[crate::db::AgentSlot],
    ) -> Result<Vec<PaneResumeOutcome>> {
        if slots.is_empty() {
            anyhow::bail!("no persisted slots to recover");
        }

        let mut ordered: Vec<crate::db::AgentSlot> = slots.to_vec();
        ordered.sort_by_key(|s| s.slot);

        // Recreate the session shell with its slot-0 primary pane via the normal
        // start path (restores worktree/sandbox). The slot-0 command is launched
        // fresh here and then uniformly resume-launched below, matching how
        // `resume_all_tracked_panes` treats every slot the same way.
        self.start_with_size(crate::terminal::get_size())?;

        let session_name = tmux::Session::generate_name(&self.id, &self.title);

        // Pair each slot with the new pane created for it, capturing pane ids in
        // slot order at creation time (see `rebuild_recovery_panes`). A slot
        // whose pane could not be created is paired with `None` and surfaced as a
        // per-pane error below without aborting its siblings.
        let paired = rebuild_recovery_panes(&self.title, &session_name, &ordered)?;
        tmux::refresh_session_cache();

        let now = crate::db::now_unix();
        let mut outcomes = Vec::with_capacity(paired.len());
        for (slot, maybe_pane) in &paired {
            let Some(new_pane) = maybe_pane else {
                outcomes.push(PaneResumeOutcome::Error(format!(
                    "pane creation failed for slot {} (cwd {})",
                    slot.slot, slot.cwd
                )));
                continue;
            };
            let outcome = self.resume_launch_pane(
                &slot.agent,
                &slot.native_session_id,
                new_pane,
                &slot.cwd,
                slot.slot == 0,
            );
            if let PaneResumeOutcome::Error(ref err) = outcome {
                tracing::warn!(
                    "Failed to recover pane (slot {}) for '{}': {}",
                    slot.slot,
                    self.title,
                    err
                );
            }
            if let Err(e) = store.upsert_agent_slot(
                &slot.instance_id,
                slot.slot,
                &slot.agent,
                &slot.native_session_id,
                &slot.cwd,
                new_pane,
                now,
            ) {
                tracing::error!(
                    "Failed to write back tmux_pane for slot {} of '{}': {}",
                    slot.slot,
                    self.title,
                    e
                );
            }
            outcomes.push(outcome);
        }

        // Re-pin @aoe_agent_pane to slot 0's pane (always created: the primary
        // pane) so reconcile and the `R` resume-all flow keep operating on the
        // rebuilt session.
        if let Some((_, Some(primary_pane))) = paired.first() {
            if let Err(e) = tmux::set_agent_pane_id(&session_name, primary_pane) {
                tracing::error!(
                    "Failed to re-pin @aoe_agent_pane for '{}': {}",
                    self.title,
                    e
                );
            }
        }

        self.run_auto_confirm();
        self.apply_tmux_options(&Self::current_profile());
        self.status = Status::Starting;
        self.last_start_time = Some(Instant::now());

        Ok(outcomes)
    }

    fn clear_resume_token(&mut self) {
        self.resume_token = None;
    }

    fn resolved_resume_token(&self, resume_token: Option<&str>) -> Option<String> {
        resume_token
            .map(std::string::ToString::to_string)
            .or_else(|| self.resume_token.clone())
    }

    fn apply_tmux_options(&self, profile: &str) {
        let name = tmux::Session::generate_name(&self.id, &self.title);
        self.apply_session_tmux_options(&name, &self.title, profile);
        if self.tool == "codex" {
            if let Err(e) = tmux::status_bar::ensure_codex_title_monitor(&name, &self.title) {
                tracing::debug!("Failed to refresh Codex title monitor: {}", e);
            }
        }
    }

    pub fn refresh_agent_tmux_options(&self, profile: &str) {
        self.apply_tmux_options(profile);
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
            self.workspace_info.as_ref(),
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

        if self.status == Status::Error {
            if let Some(last_check) = self.last_error_check {
                if last_check.elapsed().as_secs() < 30 {
                    return;
                }
            }
        }

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

        // --- Detect status for the primary (AoE-created) agent pane ---
        let mut primary_status: Option<Status> = None;

        // Check hook-based status first (more reliable than tmux pane parsing).
        // Only short-circuit when the hook file is fresh: a stale file means
        // the agent missed a `Stop` event (Esc, client-side slash command,
        // crash) and we must fall through to content detection instead of
        // pinning the session to the last hook-reported state.
        match crate::hooks::read_hook_status_with_freshness(&self.id) {
            Some(read) if read.fresh => {
                tracing::trace!("hook status detection '{}': {:?}", self.title, read.status);
                self.clear_spike_state();
                // Trust hook status over shell detection. Wrapper scripts (e.g.
                // Devbox, version managers) run agents via a shell process, so
                // `is_pane_running_shell()` returns true even though the agent is
                // healthy. Only check if the pane is actually dead.
                primary_status = Some(if session.is_pane_dead() {
                    Status::Error
                } else {
                    read.status
                });
            }
            Some(read) => {
                tracing::debug!(
                    "hook stale for '{}' (id={}, value={:?}, age={}s); falling through to content detection",
                    self.title,
                    self.id,
                    read.status,
                    read.age.as_secs()
                );
            }
            None => {}
        }

        let session_name = tmux::Session::generate_name(&self.id, &self.title);

        if primary_status.is_none() {
            if let Some(detected) = tmux::get_cached_pane_info(&session_name)
                .and_then(|info| tmux::status_detection::detect_status_from_title(&info.pane_title))
            {
                self.clear_spike_state();
                self.last_spinner_seen = Some(now);
                primary_status = Some(detected);
            }
        }

        if primary_status.is_none() {
            // When this is a shell session and a previous detach cached an
            // inner agent (e.g. user ran `claude` inside the shell and
            // detached), dispatch to that agent's content detector instead
            // of the shell stub. The capture uses the same cached 50-line
            // capture path as `session.detect_status` to avoid a double
            // capture on the same poll cycle.
            let inner_agent = if self.tool == "shell" {
                self.detected_inner_agent.clone()
            } else {
                None
            };

            let mut detected = if options.allow_capture {
                match inner_agent.as_deref() {
                    Some(agent) => match session.capture_pane_cached(50) {
                        Ok(content) => {
                            let fg_pid = session.get_foreground_pid();
                            tmux::status_detection::detect_status_from_content(
                                &content, agent, fg_pid,
                            )
                        }
                        Err(_) => Status::Idle,
                    },
                    None => match session.detect_status(&self.tool) {
                        Ok(status) => status,
                        Err(_) => Status::Idle,
                    },
                }
            } else {
                options.reused_status.unwrap_or(previous_status)
            };
            tracing::trace!(
                "status detection '{}' (tool={}, inner_agent={:?}, custom_cmd={}, allow_capture={}): {:?}",
                self.title,
                self.tool,
                inner_agent,
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

            // Apply shell/dead heuristics for single-pane sessions.
            // When `detected_inner_agent` is Some, we trust the detected
            // agent's content detector: a concrete `Idle` from (e.g.)
            // claude must surface as `Idle`, not be rewritten to `Unknown`
            // by the shell-tool heuristic.
            let is_single_pane = session.pane_count() <= 1;
            let is_shell_stale =
                || is_single_pane && !self.expects_shell() && session.is_pane_running_shell();
            detected = match detected {
                Status::Idle if inner_agent.is_some() => {
                    if session.is_pane_dead() {
                        Status::Error
                    } else {
                        Status::Idle
                    }
                }
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

            primary_status = Some(detected);
        }

        let primary_status = primary_status.unwrap_or(Status::Idle);

        // --- Detect status for extra (user-split) panes and aggregate ---
        let extra_pane_statuses =
            self.detect_extra_pane_statuses(&session_name, options.allow_capture);
        let aggregated = if extra_pane_statuses.is_empty() {
            primary_status
        } else {
            let mut all_statuses = vec![primary_status];
            all_statuses.extend(extra_pane_statuses);
            tmux::status_detection::aggregate_pane_statuses(&all_statuses)
        };

        self.status = self.apply_acknowledged_mapping(aggregated);
        self.last_error = None;
    }

    /// Detect status for extra (user-split) panes beyond the primary agent pane.
    /// Returns statuses only for panes identified as running a known agent (not shell).
    fn detect_extra_pane_statuses(&self, session_name: &str, allow_capture: bool) -> Vec<Status> {
        let all_panes = match tmux::get_all_cached_pane_infos(session_name) {
            Some(panes) if panes.len() > 1 => panes,
            _ => return Vec::new(),
        };

        // Skip pane index 0 (or whichever is the primary agent pane)
        let extra_panes: Vec<_> = all_panes.into_iter().skip(1).collect();
        let mut statuses = Vec::new();

        for pane_info in &extra_panes {
            let agent_type = match tmux::status_detection::detect_agent_type_from_pane(pane_info) {
                Some("shell") | None => continue,
                Some(agent) => agent,
            };

            // Title-based detection (fast, no capture needed)
            if let Some(status) =
                tmux::status_detection::detect_status_from_title(&pane_info.pane_title)
            {
                statuses.push(status);
                continue;
            }

            // Content-based detection (requires capture)
            if allow_capture {
                if let Ok(content) = tmux::Session::capture_pane_by_id(&pane_info.pane_id, 50) {
                    let status = tmux::status_detection::detect_status_from_content(
                        &content, agent_type, None,
                    );
                    statuses.push(status);
                    continue;
                }
            }

            statuses.push(Status::Idle);
        }

        statuses
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

/// Format an environment variable assignment as a shell-safe command prefix.
///
/// Uses `shell_escape` (single-quote escaping) so the value is preserved
/// verbatim when parsed by the inner `bash -c '...'` shell created by
/// `wrap_command_ignore_suspend`.
#[cfg(test)]
fn format_env_var_prefix(key: &str, value: &str, cmd: &str) -> String {
    let escaped = shell_escape(value);
    format!("{}={} {}", key, escaped, cmd)
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
/// breaking out of the outer shell wrapper.
///
/// Environment variables are exported before `exec` because `exec VAR=val cmd`
/// is not portable and fails in many shells.
/// Scan the Claude Code projects directory for the most recently modified
/// session file (`.jsonl`) whose project hash matches `project_path`. Returns
/// the UUID portion of the filename (the bare session id) or `None`.
///
/// Claude Code stores session data under `~/.claude/projects/<hash>/` where
/// `<hash>` is the absolute path with `/` replaced by `-`.
fn resolve_claude_session_from_disk(project_path: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(project_path)
        .unwrap_or_else(|_| std::path::PathBuf::from(project_path));
    let path_str = canonical.to_string_lossy();
    // Claude uses the path with `/` replaced by `-` as the project directory
    // name (the leading `-` comes from the initial `/`).
    let project_hash = path_str.replace('/', "-");
    let claude_dir = dirs::home_dir()?
        .join(".claude")
        .join("projects")
        .join(&project_hash);

    if !claude_dir.is_dir() {
        return None;
    }

    let mut best: Option<(std::time::SystemTime, String)> = None;
    if let Ok(entries) = std::fs::read_dir(&claude_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".jsonl") {
                continue;
            }
            let uuid = name_str.trim_end_matches(".jsonl").to_string();
            // Quick sanity check: Claude session ids are UUID-shaped (contains hyphens).
            if !uuid.contains('-') {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if best.as_ref().map_or(true, |(t, _)| modified > *t) {
                        best = Some((modified, uuid));
                    }
                }
            }
        }
    }

    best.map(|(_, id)| id)
}

fn wrap_command_ignore_suspend(cmd: &str) -> String {
    wrap_command_ignore_suspend_with_env(cmd, &[])
}

fn wrap_command_ignore_suspend_with_env(cmd: &str, env_vars: &[(&str, &str)]) -> String {
    let shell = crate::session::environment::user_posix_shell();
    let escaped = cmd.replace('\'', "'\\''");
    // Place env vars before the shell so they're parsed at the outer shell
    // level, avoiding quoting conflicts with the inner single-quoted string.
    let env_prefix = env_vars
        .iter()
        .map(|(k, v)| {
            let escaped_v = v.replace('\'', "'\\''");
            format!("{}='{}' ", k, escaped_v)
        })
        .collect::<String>();
    // Use login shell (-l) so version-manager PATHs (NVM, etc.) are available.
    format!(
        "{}{} -lc 'stty susp undef; exec env {}'",
        env_prefix, shell, escaped
    )
}

/// Outcome of resuming a single tracked pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneResumeOutcome {
    /// Pane respawned with a resume command built from the persisted id.
    Resumed,
    /// Pane respawned fresh (no resume flag): empty id, agent without
    /// `ResumeConfig`, or unknown agent.
    DegradedToFresh,
    /// Respawn failed; the error message is recorded for the caller.
    Error(String),
}

/// Whether a string is safe to use as a bare command token (binary name) in a
/// tmux respawn command. tmux runs the respawn argument through a shell, so a
/// recorded value with shell metacharacters must never be executed.
fn is_safe_command_token(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl Instance {
    /// Build the launch command for one tracked pane from its recorded agent and
    /// persisted native session id, decorating it with the instance's full launch
    /// context through [`build_pane_command`](Self::build_pane_command). Returns
    /// `Some((command, resumed))` where `resumed` is true only when a resume flag
    /// was appended.
    ///
    /// A known agent with an empty/invalid `native_session_id`, or one that lacks
    /// a `ResumeConfig`, degrades to a fresh launch that still carries the full
    /// launch context (not a bare binary). An unknown agent whose recorded name is
    /// a safe command token degrades to a bare-binary fresh launch (it cannot be
    /// decorated). Returns `None` when no safe command can be built (an unknown
    /// agent whose recorded name is not a safe command token, or a known agent the
    /// launch pipeline declines to launch in this configuration), so the caller
    /// can surface a per-pane error instead of executing it.
    ///
    /// The persisted id and unknown-agent name are validated because the command
    /// is ultimately run through a shell by `tmux respawn-pane`; an unvalidated
    /// value with shell metacharacters would otherwise be a command-injection
    /// vector. Only a `native_session_id` that passes `is_valid_resume_token` is
    /// ever interpolated into the resume flag.
    fn build_pane_resume_plan(
        &self,
        agent: &str,
        native_session_id: &str,
        is_primary: bool,
    ) -> Option<(String, bool)> {
        let Some(def) = crate::agents::get_agent(agent) else {
            // Unknown agent: only the recorded name can act as the binary, and
            // only if it is a safe command token; otherwise refuse to build a
            // command. Unknown agents cannot be decorated with launch context.
            return is_safe_command_token(agent).then(|| (agent.to_string(), false));
        };

        let resumed = def.resume.is_some() && is_valid_resume_token(native_session_id);
        let resume_token = resumed.then_some(native_session_id);
        let command = self.build_pane_command(def.name, resume_token, is_primary)?;
        Some((command, resumed))
    }

    /// Reusable per-pane resume-launch core (shared with cold-start recovery).
    ///
    /// Given a tracked pane's recorded agent, its persisted `native_session_id`,
    /// its `tmux_pane` target, its `cwd`, and whether it is the primary pane, kill
    /// the pane's process tree and respawn it with the command built through
    /// [`build_pane_resume_plan`](Self::build_pane_resume_plan) (full launch
    /// context plus, when a valid token is present, the resume flag). A pane with
    /// no usable resume id (empty/invalid id or an agent without a `ResumeConfig`)
    /// degrades to a fresh launch of that one pane that still carries the launch
    /// context. A pane whose agent name is unknown and not a safe command token,
    /// or whose tmux respawn fails, is returned as [`PaneResumeOutcome::Error`] so
    /// the caller can isolate per-pane failures.
    fn resume_launch_pane(
        &self,
        agent: &str,
        native_session_id: &str,
        tmux_pane: &str,
        cwd: &str,
        is_primary: bool,
    ) -> PaneResumeOutcome {
        // Build (and validate) the command before killing the pane, so a pane we
        // cannot safely respawn is left running rather than killed and abandoned.
        let Some((command, resumed)) =
            self.build_pane_resume_plan(agent, native_session_id, is_primary)
        else {
            return PaneResumeOutcome::Error(format!("unsafe or unknown agent '{agent}'"));
        };

        tmux::kill_pane_process_tree_target(tmux_pane);

        if let Err(err) = tmux::respawn_pane_target(tmux_pane, &command, cwd) {
            return PaneResumeOutcome::Error(err.to_string());
        }

        if resumed {
            PaneResumeOutcome::Resumed
        } else {
            PaneResumeOutcome::DegradedToFresh
        }
    }
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

/// Pure recoverability predicate: an instance is recoverable when it has
/// persisted slots and its tmux session is not currently alive.
fn is_recoverable_from(has_slots: bool, session_alive: bool) -> bool {
    has_slots && !session_alive
}

/// Recreate one pane per slot and pair each slot with its new pane id in slot
/// order. Slot 0 is the primary pane the start path already created (read back
/// from `@aoe_agent_pane`); slots 1..N are split off, each capturing its own id
/// at creation time. Pane ids are deliberately NOT re-listed via
/// `session_pane_ids`: that orders by `pane_index`, which diverges from creation
/// order for 3+ panes (every right-split inserts a pane next to pane 0). A slot
/// whose split fails (e.g. a recorded cwd that no longer exists) is paired with
/// `None` so its siblings still recover instead of the whole rebuild aborting.
fn rebuild_recovery_panes(
    title: &str,
    session_name: &str,
    ordered: &[crate::db::AgentSlot],
) -> Result<Vec<(crate::db::AgentSlot, Option<String>)>> {
    // Slot 0 is the single pane the start path just created. Prefer the pinned
    // `@aoe_agent_pane`, but fall back to listing the session's only pane for
    // start paths that don't pin it (the list is unambiguous before any split).
    let primary_pane = tmux::get_agent_pane_id(session_name)
        .or_else(|| {
            crate::db::reconcile::session_pane_ids(session_name)
                .into_iter()
                .next()
        })
        .ok_or_else(|| anyhow::anyhow!("recovered session '{}' has no primary pane", title))?;

    let mut paired = Vec::with_capacity(ordered.len());
    paired.push((ordered[0].clone(), Some(primary_pane)));

    for slot in ordered.iter().skip(1) {
        match tmux::split_window_right_capture_pane(session_name, &slot.cwd, "") {
            Ok(pane_id) => paired.push((slot.clone(), Some(pane_id))),
            Err(e) => {
                tracing::warn!(
                    "Failed to create recovery pane for slot {} of '{}' (cwd {}): {}",
                    slot.slot,
                    title,
                    slot.cwd,
                    e
                );
                paired.push((slot.clone(), None));
            }
        }
    }
    Ok(paired)
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

    #[test]
    fn test_yolo_envvar_command_is_quoted() {
        // EnvVar values containing JSON must be shell-escaped to prevent
        // the inner bash from expanding special characters ({, *, ").
        let result = format_env_var_prefix("OPENCODE_PERMISSION", r#"{"*":"allow"}"#, "opencode");
        assert_eq!(result, r#"OPENCODE_PERMISSION='{"*":"allow"}' opencode"#);
    }

    #[test]
    fn test_yolo_envvar_survives_suspend_wrapper() {
        // The full chain: format_env_var_prefix -> wrap_command_ignore_suspend
        // must preserve the JSON value through both quoting layers.
        // Single quotes from shell_escape are escaped by wrap_command_ignore_suspend
        // via the '\'' technique, which correctly round-trips through the shell.
        let cmd = format_env_var_prefix("OPENCODE_PERMISSION", r#"{"*":"allow"}"#, "opencode");
        let wrapped = wrap_command_ignore_suspend(&cmd);
        // The inner single quotes from shell_escape become '\'' in the outer wrapper
        assert!(
            wrapped.contains(r#"OPENCODE_PERMISSION='\''{"*":"allow"}'\'' opencode"#),
            "wrapped command should contain the escaped env var assignment: {}",
            wrapped,
        );
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
        let shell = crate::session::environment::user_posix_shell();
        assert_eq!(
            wrap_command_ignore_suspend("opencode"),
            format!("{shell} -lc 'stty susp undef; exec env opencode'")
        );
    }

    #[test]
    fn test_wrap_command_ignore_suspend_with_env() {
        let shell = crate::session::environment::user_posix_shell();
        let result = wrap_command_ignore_suspend_with_env(
            "opencode",
            &[("OPENCODE_PERMISSION", r#"{"*":"allow"}"#)],
        );
        // Env vars are placed before the shell, not inside the single-quoted string
        assert_eq!(
            result,
            format!(
                r#"OPENCODE_PERMISSION='{{"*":"allow"}}' {shell} -lc 'stty susp undef; exec env opencode'"#
            )
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
        let shell = crate::session::environment::user_posix_shell();
        assert!(
            cmd.contains(
                &format!("{shell} -lc 'stty susp undef; exec env claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11 --model sonnet --dangerously-skip-permissions'")
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

        let shell = crate::session::environment::user_posix_shell();
        assert_eq!(
            cmd,
            format!("{shell} -lc 'stty susp undef; exec env codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded --model gpt-5 --dangerously-bypass-approvals-and-sandbox'")
        );
    }

    #[test]
    fn test_cross_agent_team_flag_appended_when_enabled() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains(
                "--dangerously-load-development-channels server:cross-agent-teams-channel"
            ),
            "expected dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_flag_absent_when_disabled() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = false;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("--dangerously-load-development-channels"),
            "did not expect dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_coexists_with_yolo() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "expected yolo flag, got {cmd}"
        );
        assert!(
            cmd.contains(
                "--dangerously-load-development-channels server:cross-agent-teams-channel"
            ),
            "expected dev-channels flag, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_custom_channel() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;
        inst.cross_agent_team_channel = "server:my-channel".to_string();

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            cmd.contains("--dangerously-load-development-channels server:my-channel"),
            "expected custom channel, got {cmd}"
        );
    }

    #[test]
    fn test_cross_agent_team_no_token_injection() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("CROSS_AGENT_TEAMS_MCP_TOKEN"),
            "token must be inherited from environment, not injected: {cmd}"
        );
    }

    #[test]
    fn test_is_auto_confirm_screen_dev_channels() {
        let screen = "  WARNING: Loading development channels\n  ❯ 1. I am using this for local development\n    2. Exit";
        assert!(is_auto_confirm_screen(screen));
    }

    #[test]
    fn test_is_auto_confirm_screen_trust_folder() {
        let screen = " Quick safety check: Is this a project you created or one you trust?\n ❯ 1. Yes, I trust this folder";
        assert!(is_auto_confirm_screen(screen));
    }

    #[test]
    fn test_is_auto_confirm_screen_negative() {
        let screen = "Welcome to Claude Code\n> how can I help?";
        assert!(!is_auto_confirm_screen(screen));
    }

    #[test]
    fn test_is_auto_confirm_screen_with_ansi_per_word_coloring() {
        // Claude colors the warning title per word; `tmux capture-pane -e`
        // interleaves SGR codes, splitting the phrase. Stripping must restore it.
        let screen = "\u{1b}[39m  \u{1b}[1m\u{1b}[38;5;211mWARNING:\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mLoading\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mdevelopment\u{1b}[0m \u{1b}[1m\u{1b}[38;5;211mchannels\u{1b}[0m";
        assert!(
            !screen.contains("Loading development channels"),
            "raw -e capture should not contain the contiguous phrase"
        );
        assert!(
            is_auto_confirm_screen(screen),
            "after stripping ANSI the phrase must match"
        );
    }

    #[test]
    fn test_strip_ansi_basic() {
        assert_eq!(strip_ansi("\u{1b}[1mhi\u{1b}[0m there"), "hi there");
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn test_run_auto_confirm_noop_for_non_cross_agent_team() {
        // Must not panic or spawn work when the mode is off.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.cross_agent_team = false;
        inst.run_auto_confirm();

        // Also a no-op for non-claude even if the flag is set.
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;
        inst.run_auto_confirm();
    }

    #[test]
    fn test_cross_agent_team_ignored_for_non_claude() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        inst.cross_agent_team = true;

        let cmd = inst.build_agent_command(None).unwrap();
        assert!(
            !cmd.contains("--dangerously-load-development-channels"),
            "dev-channels flag should be claude-only, got {cmd}"
        );
    }

    #[test]
    fn test_build_agent_command_shell_starts_in_project_path() {
        let shell = crate::session::environment::user_posix_shell();
        let mut inst = Instance::new("test", "/tmp/expected path");
        inst.tool = "shell".to_string();
        inst.command = shell.clone();

        let cmd = inst.build_agent_command(None).unwrap();
        let escaped_dir = shell_escape("/tmp/expected path");
        let expected_inner =
            format!("cd {escaped_dir} && stty susp undef; exec {shell}").replace('\'', "'\\''");
        let expected = format!("{shell} -lc '{expected_inner}'");

        assert_eq!(cmd, expected);
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
    fn test_build_pane_resume_plan_claude_appends_resume_flag() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_codex_uses_resume_subcommand() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "codex".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("codex", "019d1af9-a899-7df1-8f7d-a244126e5ded", true)
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("codex resume 019d1af9-a899-7df1-8f7d-a244126e5ded"),
            "expected resume subcommand, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_empty_id_restarts_fresh() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst.build_pane_resume_plan("claude", "", true).unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("--resume"),
            "expected no resume flag, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_invalid_id_restarts_fresh() {
        // A persisted id with shell metacharacters must never be substituted
        // into the command; it degrades to a fresh restart instead.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "abc; rm -rf ~", true)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("--resume"),
            "expected no resume flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("rm -rf"),
            "unsafe id must not be interpolated: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_agent_without_resume_config_restarts_fresh() {
        // gemini has no ResumeConfig -> fresh launch even with a persisted id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "gemini".to_string();
        let (cmd, resumed) = inst
            .build_pane_resume_plan("gemini", "gemini-sess-0", true)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("resume"),
            "expected no resume flag, got: {cmd}"
        );
        assert!(
            cmd.contains(crate::agents::get_agent("gemini").unwrap().binary),
            "expected gemini binary, got: {cmd}"
        );
    }

    #[test]
    fn test_build_pane_resume_plan_unknown_safe_agent_uses_recorded_name_fresh() {
        // An unknown but safe agent name cannot be decorated; it degrades to a
        // bare-binary fresh launch.
        let inst = Instance::new("test", "/tmp/test");
        let (cmd, resumed) = inst
            .build_pane_resume_plan("mystery", "some-id", false)
            .unwrap();
        assert!(!resumed);
        assert_eq!(cmd, "mystery");
    }

    #[test]
    fn test_build_pane_resume_plan_unsafe_unknown_agent_is_rejected() {
        // An unknown agent name with shell metacharacters must not be executed.
        let inst = Instance::new("test", "/tmp/test");
        assert!(inst
            .build_pane_resume_plan("evil; rm -rf ~", "some-id", false)
            .is_none());
    }

    fn sandboxed_instance(tool: &str) -> Instance {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = tool.to_string();
        inst.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: None,
            image: "test-image".to_string(),
            container_name: "test".to_string(),
            created_at: None,
            extra_env: None,
            custom_instruction: None,
        });
        inst
    }

    // --- Slot-resume launch-context preservation (fix-resume-preserves-launch-context) ---

    #[test]
    fn test_slot_resume_yolo_cliflag_keeps_flag_and_resume_token() {
        // A YOLO CliFlag agent (claude) resumed via the slot path must carry both
        // the YOLO flag and the resume flag built from native_session_id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;

        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(resumed, "expected a resume plan, got fresh");
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag from native_session_id, got: {cmd}"
        );
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "expected YOLO CliFlag, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_yolo_envvar_sets_env_var() {
        // A YOLO EnvVar agent resumed via the slot path must set the YOLO env var.
        // opencode is sandbox-only on the real host path, so the host EnvVar branch
        // is reached here through a command override equal to the binary (a real,
        // reachable configuration that still exercises the EnvVar decoration).
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "opencode".to_string();
        inst.command = "opencode".to_string();
        inst.yolo_mode = true;

        let cmd = inst
            .build_pane_command("opencode", None, true)
            .expect("opencode command override should build");
        assert!(
            cmd.contains("OPENCODE_PERMISSION="),
            "expected YOLO env var, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_hook_agent_sets_instance_id() {
        // A hook-config agent (claude) resumed via the slot path must carry
        // AOE_INSTANCE_ID set to the instance id.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        let id = inst.id.clone();

        let (cmd, _resumed) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(
            cmd.contains(&format!("AOE_INSTANCE_ID='{id}'")),
            "expected AOE_INSTANCE_ID env, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_sandboxed_is_docker_wrapped() {
        // A sandboxed instance resumed via the slot path must be docker-exec
        // wrapped into the instance's container, not a bare host binary.
        let inst = sandboxed_instance("claude");
        let container = DockerContainer::generate_name(&inst.id);

        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(resumed);
        assert!(
            cmd.contains("exec -it") && cmd.contains(&container),
            "expected docker exec into {container}, got: {cmd}"
        );
        assert!(
            cmd.contains("claude --resume 4dc7a3c8-934e-40c1-95f8-8b00fe11cf11"),
            "expected resume flag inside container command, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_non_yolo_has_no_yolo_flag_or_env() {
        // A non-YOLO instance resumed via the slot path must not gain any YOLO
        // flag or YOLO env var.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = false;

        let (cmd, _resumed) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(
            !cmd.contains("--dangerously-skip-permissions"),
            "non-YOLO must not carry YOLO flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("OPENCODE_PERMISSION"),
            "non-YOLO must not carry YOLO env, got: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_degraded_fresh_keeps_launch_context() {
        // A pane with no usable resume token (invalid native_session_id) must
        // still launch fresh WITH full launch context (YOLO flag, hook env),
        // never a bare binary.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;
        let id = inst.id.clone();

        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "not a valid; token", true)
            .unwrap();
        assert!(!resumed, "invalid token must degrade to fresh");
        assert!(
            !cmd.contains("--resume"),
            "degraded-fresh must not carry a resume flag, got: {cmd}"
        );
        assert!(
            !cmd.contains("not a valid"),
            "unsafe token must not be interpolated, got: {cmd}"
        );
        assert!(
            cmd.contains("--dangerously-skip-permissions"),
            "degraded-fresh must still carry YOLO flag, got: {cmd}"
        );
        assert!(
            cmd.contains(&format!("AOE_INSTANCE_ID='{id}'")),
            "degraded-fresh must still carry hook env, got: {cmd}"
        );
        let binary = crate::agents::get_agent("claude").unwrap().binary;
        assert_ne!(cmd, binary, "degraded-fresh must not be a bare binary");
    }

    #[test]
    fn test_slot_resume_injection_guard_intact() {
        // An unsafe/unknown slot agent name is refused (None); an invalid resume
        // token degrades to fresh without interpolating the raw value.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();

        assert!(
            inst.build_pane_resume_plan("evil; rm -rf ~", "some-id", true)
                .is_none(),
            "unsafe agent name must be refused"
        );

        let (cmd, resumed) = inst
            .build_pane_resume_plan("claude", "abc; rm -rf ~", true)
            .unwrap();
        assert!(!resumed);
        assert!(
            !cmd.contains("rm -rf"),
            "invalid resume token must not be interpolated: {cmd}"
        );
    }

    #[test]
    fn test_slot_resume_heterogeneous_panes_apply_own_yolo_variant() {
        // A YOLO instance whose slots record different agents must apply each
        // pane's own YoloMode variant: claude (CliFlag) gets the flag; pi
        // (AlwaysYolo) gets neither a flag nor an env var.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.yolo_mode = true;

        let (primary, _) = inst
            .build_pane_resume_plan("claude", "4dc7a3c8-934e-40c1-95f8-8b00fe11cf11", true)
            .unwrap();
        assert!(
            primary.contains("--dangerously-skip-permissions"),
            "claude pane must carry its CliFlag, got: {primary}"
        );

        let (secondary, _) = inst.build_pane_resume_plan("pi", "ignored", false).unwrap();
        assert!(
            !secondary.contains("--dangerously-skip-permissions"),
            "pi (AlwaysYolo) pane must not carry claude's flag, got: {secondary}"
        );
        assert!(
            secondary.contains("pi"),
            "pi pane must launch the pi binary, got: {secondary}"
        );
    }

    #[test]
    fn test_is_recoverable_slots_and_dead_session() {
        // Has persisted slots AND tmux session dead => recoverable.
        assert!(is_recoverable_from(true, false));
    }

    #[test]
    fn test_is_recoverable_live_session_never_recoverable() {
        // Live session is never recoverable regardless of slots.
        assert!(!is_recoverable_from(true, true));
    }

    #[test]
    fn test_is_recoverable_no_slots_not_recoverable() {
        // No persisted slots => not recoverable even when the session is dead.
        assert!(!is_recoverable_from(false, false));
        assert!(!is_recoverable_from(false, true));
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
    fn test_extra_args_without_command_override_is_not_a_command_override() {
        let mut inst = Instance::new("test", "/tmp/test");
        inst.tool = "claude".to_string();
        inst.extra_args = "--model sonnet".to_string();
        assert!(!inst.has_command_override());
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
    fn test_restart_in_flight_is_runtime_only() {
        let mut inst = Instance::new("Test", "/tmp/test");
        inst.restart_in_flight = true;

        let json = serde_json::to_string(&inst).unwrap();
        assert!(!json.contains("restart_in_flight"));
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

    // --- Fork session tests ----------------------------------------------

    fn parent_instance(tool: &str, token: Option<&str>) -> Instance {
        let mut inst = Instance::new("parent", "/tmp/project");
        inst.tool = tool.to_string();
        inst.group_path = "work".to_string();
        inst.extra_args = "--verbose".to_string();
        inst.yolo_mode = true;
        inst.resume_token = token.map(|s| s.to_string());
        inst
    }

    #[test]
    fn test_create_fork_inherits_parent_fields() {
        let parent = parent_instance("claude", Some("abc-123"));
        let fork = parent
            .create_fork("my-fork".to_string(), Some("experiments".to_string()))
            .expect("fork should succeed");

        assert_ne!(fork.id, parent.id);
        assert_eq!(fork.id.len(), parent.id.len());
        assert_eq!(fork.title, "my-fork");
        assert_eq!(fork.project_path, parent.project_path);
        assert_eq!(fork.tool, parent.tool);
        assert_eq!(fork.extra_args, parent.extra_args);
        assert_eq!(fork.yolo_mode, parent.yolo_mode);
        assert_eq!(fork.group_path, "experiments");
        assert_eq!(fork.parent_session_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(fork.fork_pending.as_deref(), Some("abc-123"));
        // Runtime state is reset.
        assert!(fork.resume_token.is_none());
        assert!(fork.last_error.is_none());
        assert_eq!(fork.status, Status::Idle);
        assert!(!fork.acknowledged);
    }

    #[test]
    fn test_create_fork_defaults_to_parent_group() {
        let parent = parent_instance("codex", Some("xyz"));
        let fork = parent
            .create_fork("sibling".to_string(), None)
            .expect("fork should succeed");
        assert_eq!(fork.group_path, parent.group_path);
    }

    #[test]
    fn test_create_fork_rejects_unsupported_tool() {
        let parent = parent_instance("gemini", Some("ignored"));
        let err = parent
            .create_fork("bad".to_string(), None)
            .expect_err("gemini does not support forking");
        let msg = err.to_string();
        assert!(
            msg.contains("Fork is not supported"),
            "expected unsupported-tool error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_rejects_missing_codex_token() {
        let parent = parent_instance("codex", None);
        let err = parent
            .create_fork("too-early".to_string(), None)
            .expect_err("codex without a resume token should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("No active codex session"),
            "expected missing-token error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_claude_no_token_falls_back_to_disk() {
        // Claude without resume_token should attempt disk scan.
        // In the test environment there's no Claude project directory,
        // so it should fail with a "No active Claude session" error.
        let parent = parent_instance("claude", None);
        let err = parent
            .create_fork("no-disk".to_string(), None)
            .expect_err("claude without token or disk session should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("No active Claude session"),
            "expected missing-session error, got: {msg}"
        );
    }

    #[test]
    fn test_create_fork_rejects_command_override() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.command = "claude --some-weird-wrapper".to_string();
        let err = parent
            .create_fork("nope".to_string(), None)
            .expect_err("command override should block fork");
        assert!(err.to_string().contains("custom command override"));
    }

    #[test]
    fn test_create_fork_clears_worktree_cleanup_flag() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.worktree_info = Some(WorktreeInfo {
            branch: "main".to_string(),
            main_repo_path: "/tmp/project".to_string(),
            managed_by_aoe: true,
            created_at: Utc::now(),
            cleanup_on_delete: true,
        });
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let wt = fork.worktree_info.expect("worktree inherited");
        assert!(!wt.cleanup_on_delete);
        assert_eq!(wt.branch, "main");
    }

    #[test]
    fn test_create_fork_generates_new_container_name() {
        let mut parent = parent_instance("claude", Some("abc"));
        parent.sandbox_info = Some(SandboxInfo {
            enabled: true,
            container_id: Some("parent-container-id".to_string()),
            image: "ubuntu:latest".to_string(),
            container_name: DockerContainer::generate_name(&parent.id),
            created_at: Some(Utc::now()),
            extra_env: None,
            custom_instruction: None,
        });
        let parent_container_name = parent.sandbox_info.as_ref().unwrap().container_name.clone();

        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let sandbox = fork.sandbox_info.expect("sandbox inherited");
        assert_ne!(sandbox.container_name, parent_container_name);
        assert_eq!(
            sandbox.container_name,
            DockerContainer::generate_name(&fork.id)
        );
        assert!(sandbox.container_id.is_none());
    }

    #[test]
    fn test_build_base_tool_command_uses_fork_template_for_claude() {
        let parent = parent_instance("claude", Some("parent-uuid"));
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let agent = crate::agents::get_agent("claude");
        let cmd = fork.build_base_pane_command(agent, None, true);
        assert!(
            cmd.contains("claude --resume parent-uuid --fork-session"),
            "expected claude fork command, got: {cmd}"
        );
    }

    #[test]
    fn test_build_base_tool_command_uses_fork_template_for_codex() {
        let parent = parent_instance("codex", Some("parent-uuid"));
        let fork = parent.create_fork("f".to_string(), None).unwrap();
        let agent = crate::agents::get_agent("codex");
        let cmd = fork.build_base_pane_command(agent, None, true);
        assert!(
            cmd.contains("codex fork parent-uuid"),
            "expected codex fork command, got: {cmd}"
        );
    }

    #[test]
    fn test_build_base_tool_command_resume_beats_fork_pending() {
        // Once AoE has captured a real resume token, fork_pending must not
        // override it. This guards the second-launch transition.
        let parent = parent_instance("claude", Some("old-parent-uuid"));
        let mut fork = parent.create_fork("f".to_string(), None).unwrap();
        fork.resume_token = Some("new-fork-uuid".to_string());
        let agent = crate::agents::get_agent("claude");
        let cmd = fork.build_base_pane_command(agent, Some("new-fork-uuid"), true);
        assert!(
            cmd.contains("--resume new-fork-uuid") && !cmd.contains("--fork-session"),
            "expected plain resume command, got: {cmd}"
        );
    }

    #[test]
    fn test_deserialize_instance_defaults_fork_pending_none() {
        let json = r#"{
            "id":"abc","title":"t","project_path":"/p","command":"",
            "created_at":"2020-01-01T00:00:00Z"
        }"#;
        let inst: Instance = serde_json::from_str(json).expect("parseable");
        assert!(inst.fork_pending.is_none());
    }

    // --- Hook status freshness gating tests ---
    //
    // These tests exercise the decision that `update_status_with_options` uses
    // to choose between trusting the hook file and falling through to
    // content-based detection. The gate itself lives in
    // `crate::hooks::read_hook_status_with_freshness`; here we verify that
    // fresh files produce an authoritative read, stale files fall through,
    // and missing files are reported as absent.
    //
    // We cannot drive the full `update_status_with_options` path in a unit
    // test because it requires a live tmux session, so the gate is the
    // smallest unit that captures the behavior change introduced by this
    // feature.

    fn write_hook_status_for(instance_id: &str, value: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new("/tmp/aoe-hooks").join(instance_id);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("status");
        std::fs::write(&path, value).unwrap();
        path
    }

    fn set_mtime_seconds_ago(path: &std::path::Path, seconds: u64) {
        // Shell out to `touch -t` which is POSIX and works on macOS and Linux
        // without adding a new crate dependency for tests.
        use chrono::{Local, TimeZone};
        let target = Local::now() - chrono::Duration::seconds(seconds as i64);
        let stamp = Local
            .timestamp_opt(target.timestamp(), 0)
            .single()
            .unwrap()
            .format("%Y%m%d%H%M.%S")
            .to_string();
        let status = std::process::Command::new("touch")
            .args(["-t", &stamp, path.to_str().unwrap()])
            .status()
            .expect("touch should run");
        assert!(status.success(), "touch -t failed for {:?}", path);
    }

    #[test]
    fn test_update_status_fresh_hook_running_short_circuits() {
        let id = "test_upd_fresh_hook_running";
        let path = write_hook_status_for(id, "running");
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(read.fresh, "just-written file must be fresh");
        assert_eq!(read.status, Status::Running);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_stale_hook_running_falls_through() {
        let id = "test_upd_stale_hook_running";
        let path = write_hook_status_for(id, "running");
        set_mtime_seconds_ago(&path, 120);
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(!read.fresh, "file older than window must be stale");
        assert_eq!(read.status, Status::Running);
        assert!(read.age.as_secs() >= 60, "age should reflect mtime");
        // The poller is expected to ignore `read.status` when !fresh and let
        // content detection drive the final result.
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_stale_hook_waiting_falls_through() {
        let id = "test_upd_stale_hook_waiting";
        let path = write_hook_status_for(id, "waiting");
        set_mtime_seconds_ago(&path, 3600);
        let read = crate::hooks::read_hook_status_with_freshness(id).expect("file present");
        assert!(!read.fresh);
        assert_eq!(read.status, Status::Waiting);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn test_update_status_missing_hook_file_is_absent() {
        // With no hook file at all, the reader reports None so the poller
        // will proceed to title fast-path / content detection unchanged.
        assert!(
            crate::hooks::read_hook_status_with_freshness("test_upd_missing_hook_file").is_none()
        );
    }

    // --- detected_inner_agent tests ----------------------------------------
    //
    // These cover the shell-session-in-memory inner-agent discovery field.
    // The end-to-end status dispatch path requires a live tmux session so
    // the full `update_status_with_options` is exercised in e2e tests; here
    // we focus on the pure in-process decisions that gate behavior on the
    // field's value.

    #[test]
    fn test_detected_inner_agent_default_is_none() {
        let inst = Instance::new("test", "/tmp/test");
        assert!(inst.detected_inner_agent.is_none());
    }

    /// Simulates the attach-return path's normalization logic. The real
    /// writer in `src/tui/app.rs::attach_session` uses the same rule:
    /// `Some("shell") | None` → `None`, `Some(x)` → `Some(x.to_string())`.
    fn normalize_detected_agent(detected: Option<&str>) -> Option<String> {
        match detected {
            Some("shell") | None => None,
            Some(agent) => Some(agent.to_string()),
        }
    }

    #[test]
    fn test_detected_inner_agent_normalization_clears_on_shell_or_none() {
        assert_eq!(normalize_detected_agent(Some("shell")), None);
        assert_eq!(normalize_detected_agent(None), None);
    }

    #[test]
    fn test_detected_inner_agent_normalization_stores_known_agents() {
        assert_eq!(
            normalize_detected_agent(Some("claude")),
            Some("claude".to_string())
        );
        assert_eq!(
            normalize_detected_agent(Some("codex")),
            Some("codex".to_string())
        );
    }

    #[test]
    fn test_detected_inner_agent_not_serialized() {
        // Field MUST be `#[serde(skip)]` so a round trip to disk drops it.
        let mut inst = Instance::new("test", "/tmp/test");
        inst.detected_inner_agent = Some("claude".to_string());
        let json = serde_json::to_string(&inst).expect("serialize");
        assert!(
            !json.contains("detected_inner_agent"),
            "field leaked into serialized JSON: {json}"
        );
        let restored: Instance = serde_json::from_str(&json).expect("deserialize");
        assert!(
            restored.detected_inner_agent.is_none(),
            "deserialize must produce None, got {:?}",
            restored.detected_inner_agent
        );
    }

    /// Captures the dispatch rule used by `update_status_with_options`: when
    /// `tool == "shell"` and `detected_inner_agent = Some(X)`, content
    /// detection routes through `detect_status_from_content(_, X, _)`. This
    /// verifies the routing plus concrete running/idle fixtures for claude.
    #[test]
    fn test_detect_dispatch_uses_inner_agent_for_claude_running() {
        let inst = {
            let mut i = Instance::new("test", "/tmp/test");
            i.tool = "shell".to_string();
            i.detected_inner_agent = Some("claude".to_string());
            i
        };
        let agent = inst.detected_inner_agent.as_deref().expect("set above");
        let content = "Some output\n\u{280b} Working on task...\n";
        let status =
            crate::tmux::status_detection::detect_status_from_content(content, agent, None);
        assert_eq!(status, Status::Running);
    }

    #[test]
    fn test_detect_dispatch_uses_inner_agent_for_claude_idle() {
        let inst = {
            let mut i = Instance::new("test", "/tmp/test");
            i.tool = "shell".to_string();
            i.detected_inner_agent = Some("claude".to_string());
            i
        };
        let agent = inst.detected_inner_agent.as_deref().expect("set above");
        let content = "Done.\n\n\u{276f} \n";
        let status =
            crate::tmux::status_detection::detect_status_from_content(content, agent, None);
        assert_eq!(status, Status::Idle);
    }

    /// Isolates the post-detection status rewrite. Task 3.3 requires that
    /// a concrete `Idle` from a real agent detector (when the session has
    /// `detected_inner_agent = Some(_)`) surface as `Idle`, NOT be
    /// rewritten to `Unknown` by the shell/custom-command heuristic.
    fn apply_idle_rewrite(
        detected: Status,
        inner_agent_set: bool,
        has_custom_command: bool,
        pane_dead: bool,
        shell_stale: bool,
    ) -> Status {
        match detected {
            Status::Idle if inner_agent_set => {
                if pane_dead {
                    Status::Error
                } else {
                    Status::Idle
                }
            }
            Status::Idle if has_custom_command => {
                if pane_dead || shell_stale {
                    Status::Error
                } else {
                    Status::Unknown
                }
            }
            Status::Idle if pane_dead || shell_stale => Status::Error,
            other => other,
        }
    }

    #[test]
    fn test_idle_rewrite_preserves_idle_for_detected_agent() {
        assert_eq!(
            apply_idle_rewrite(Status::Idle, true, true, false, false),
            Status::Idle,
        );
    }

    #[test]
    fn test_idle_rewrite_dead_pane_with_detected_agent_becomes_error() {
        assert_eq!(
            apply_idle_rewrite(Status::Idle, true, true, true, false),
            Status::Error,
        );
    }

    #[test]
    fn test_idle_rewrite_shell_without_detected_agent_becomes_unknown() {
        // Current shell-session default: `has_custom_command` true, no
        // detected inner agent, alive pane, not shell-stale → Unknown.
        assert_eq!(
            apply_idle_rewrite(Status::Idle, false, true, false, false),
            Status::Unknown,
        );
    }

    #[test]
    fn test_idle_rewrite_agent_running_passes_through() {
        // Non-Idle statuses are untouched regardless of flags.
        assert_eq!(
            apply_idle_rewrite(Status::Running, true, true, false, false),
            Status::Running,
        );
        assert_eq!(
            apply_idle_rewrite(Status::Waiting, false, true, false, false),
            Status::Waiting,
        );
    }

    #[test]
    fn test_detected_inner_agent_not_mutated_by_update_status_short_circuit() {
        // The polling path must never write to `detected_inner_agent`.
        // Here we drive `update_status_with_options` down its early-exit
        // branch (Stopped status) and confirm the field is preserved.
        let mut inst = Instance::new("test_no_mutate_on_stopped", "/tmp/test");
        inst.tool = "shell".to_string();
        inst.detected_inner_agent = Some("claude".to_string());
        inst.status = Status::Stopped;
        inst.update_status_with_options(StatusUpdateOptions::default());
        assert_eq!(inst.detected_inner_agent.as_deref(), Some("claude"));
    }
}
