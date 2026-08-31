//! Centralized agent registry.
//!
//! All per-agent metadata lives here. Adding a new agent means adding one
//! `AgentDef` entry to `AGENTS` and writing a status detection function.

use crate::session::Status;
use crate::tmux::status_detection;

/// How to check whether an agent binary is installed on the host.
pub enum DetectionMethod {
    /// Run `which <binary>` and check exit code.
    Which(&'static str),
    /// Run `<binary> <arg>` and check that it doesn't error (e.g. `vibe --version`).
    RunWithArg(&'static str, &'static str),
}

/// How to enable YOLO / auto-approve mode for an agent.
pub enum YoloMode {
    /// Append a CLI flag (e.g. `--dangerously-skip-permissions`).
    CliFlag(&'static str),
    /// Set an environment variable (name, value).
    EnvVar(&'static str, &'static str),
    /// Agent always runs in YOLO mode with no opt-in needed (e.g. pi).
    AlwaysYolo,
}

/// A single hook event that AoE registers in an agent's settings file.
pub struct HookEvent {
    /// Event name as the agent expects it (e.g. `"PreToolUse"` for Claude Code).
    pub name: &'static str,
    /// Optional matcher pattern (e.g. `"permission_prompt|elicitation_dialog"`).
    pub matcher: Option<&'static str>,
    /// AoE status to write when this event fires (`"running"`, `"idle"`, `"waiting"`).
    pub status: Option<&'static str>,
}

/// Configuration for installing status-detection hooks into an agent's settings file.
///
/// An agent belongs here only when its hooks can be relied on to run in the
/// agent's own process, inheriting the pane's environment. Codex is the
/// counterexample and has no entry: its `--remote` clients execute hooks in a
/// shared app-server whose environment was frozen at daemon start, so a hook
/// there sees another pane's `$TMUX_PANE` and no `$AOE_INSTANCE_ID` at all.
/// Codex panes are tracked from its rollout files instead (`db::codex_rollout`)
/// and its status comes from content detection.
pub struct AgentHookConfig {
    /// Path relative to the home dir where the agent's settings live
    /// (e.g. `.claude/settings.json`).
    pub settings_rel_path: &'static str,
    /// Hook events to register (status transitions).
    pub events: &'static [HookEvent],
}

/// Who owns the server an exact-session runtime attaches to.
///
/// AoE prepares the conversation of an exact-session agent before its pane
/// starts, so every launch, restart and identity path has to know whether the
/// server behind that conversation is AoE's to end. Expressing it here is what
/// keeps those paths from asking "is this agent named opencode".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactSessionRuntime {
    /// AoE starts a per-pane loopback server and terminates it with the pane.
    /// The base URL is itself a pane-distinguishing dimension.
    OwnedServer,
    /// A user-owned singleton server AoE only discovers and connects to. Every
    /// pane shares one base URL, so only the session id tells them apart, and
    /// AoE must never start or terminate it.
    SharedServer,
}

/// Agent-specific configuration for graceful exit and resume-aware restarts.
#[derive(Debug, Clone, Copy)]
pub struct ResumeConfig {
    /// Exit key groups sent one group per tick during graceful restart.
    pub exit_sequence: &'static [&'static [&'static str]],
    /// Regex pattern with a single capture group for the resume token.
    pub resume_pattern: &'static str,
    /// CLI flag or subcommand template with `{}` placeholder for the token.
    pub resume_flag: &'static str,
    /// Graceful exit timeout before falling back to a fresh restart.
    pub timeout_secs: u64,
}

/// Everything we know about a single agent CLI.
pub struct AgentDef {
    /// Canonical name: `"claude"`, `"opencode"`, etc.
    pub name: &'static str,
    /// Binary to invoke (usually same as name).
    pub binary: &'static str,
    /// Alternative substrings recognised by `resolve_tool_name` (e.g. `"open-code"`).
    pub aliases: &'static [&'static str],
    /// How to detect availability on the host.
    pub detection: DetectionMethod,
    /// YOLO/auto-approve configuration.
    pub yolo: Option<YoloMode>,
    /// CLI flag template for custom instruction injection.
    /// `{}` is replaced with the shell-escaped instruction text.
    pub instruction_flag: Option<&'static str>,
    /// Arguments every AoE-built launch command for this agent must carry, on
    /// every launch path. Rides the command line only; never written to the
    /// agent's user configuration. Codex uses this to suppress its blocking
    /// startup update menu, which turns any stray Enter into an agent-killing
    /// `npm install -g` update in a managed pane.
    pub fixed_args: &'static [&'static str],
    /// If true, `builder.rs` sets `instance.command = binary` for this agent.
    pub set_default_command: bool,
    /// If true, the agent can be launched directly on the host (non-sandboxed).
    pub supports_host_launch: bool,
    /// Status detection function pointer. Takes raw (non-lowercased) pane content.
    pub detect_status: fn(&str) -> Status,
    /// Environment variables always injected into the container for this agent.
    pub container_env: &'static [(&'static str, &'static str)],
    /// Hook configuration for file-based status detection. If set, AoE installs
    /// hooks into the agent's settings file so status is written to a file instead
    /// of being parsed from tmux pane content.
    pub hook_config: Option<AgentHookConfig>,
    /// Graceful-exit resume support for restart flows.
    pub resume: Option<ResumeConfig>,
    /// CLI flag template for the agent's native fork-session command, with `{}` as
    /// a placeholder for the parent agent's session token.
    ///
    /// When `Instance::fork_pending` is set and no resume token is present,
    /// `build_base_tool_command` substitutes the parent token into this template and
    /// appends it to the agent binary. Examples:
    /// - Claude:   `--resume {} --fork-session`
    /// - Codex:    `fork {}`
    /// - OpenCode: `--session {} --fork`
    ///
    /// `None` means the agent does not support forking through AoE.
    pub fork_template: Option<&'static str>,
    /// CLI flag template for pre-allocating a session UUID at launch time,
    /// with `{}` as placeholder for the UUID (e.g. `--session-id {}`).
    /// When set, AoE generates a UUID before starting the agent and passes
    /// it via this flag so the conversation identity is known from the start
    /// (needed for reliable fork without post-hoc session discovery).
    pub session_id_flag: Option<&'static str>,
    /// Whether this agent sets its own terminal/pane title via OSC 0.
    /// When false, AoE manages the pane title based on detected status.
    pub sets_own_title: bool,
    /// Set when AoE prepares this agent's conversation before the pane starts,
    /// stating who owns the server that conversation lives on. `None` means the
    /// agent picks its own conversation and AoE only passes a resume flag.
    pub exact_session_runtime: Option<ExactSessionRuntime>,
    /// Whether a pane running this agent can join a Cross Agent Team.
    pub supports_cross_agent_team: bool,
}

impl AgentDef {
    /// Whether AoE prepares this agent's exact conversation before launch.
    pub fn uses_exact_session_runtime(&self) -> bool {
        self.exact_session_runtime.is_some()
    }

    /// Whether the xats identity key may be injected into this agent's pane
    /// environment. An exact-session agent is prepared by AoE, which holds the
    /// key on the durable slot and passes it only to the xats control plane;
    /// a shared-server agent additionally leaks the key to every sibling agent
    /// on that server if it reaches the pane at all.
    pub fn identity_key_in_pane_env(&self) -> bool {
        self.exact_session_runtime.is_none()
    }
}

/// The exact-session runtime shape of `tool`, if it has one.
pub fn exact_session_runtime(tool: &str) -> Option<ExactSessionRuntime> {
    get_agent(tool).and_then(|agent| agent.exact_session_runtime)
}

/// Whether `tool` names an agent that can join a Cross Agent Team.
pub fn supports_cross_agent_team(tool: &str) -> bool {
    get_agent(tool).is_some_and(|agent| agent.supports_cross_agent_team)
}

/// Whether a pane running `tool` may carry the xats identity key in its
/// environment. An unknown tool is launched undecorated, so it answers `true`
/// the way every non-exact-session agent does.
pub fn identity_key_in_pane_env(tool: &str) -> bool {
    get_agent(tool).map_or(true, AgentDef::identity_key_in_pane_env)
}

/// Hook events shared by Claude Code and Cursor CLI.
const CLAUDE_CURSOR_HOOK_EVENTS: &[HookEvent] = &[
    HookEvent {
        name: "PreToolUse",
        matcher: None,
        status: Some("running"),
    },
    HookEvent {
        name: "UserPromptSubmit",
        matcher: None,
        status: Some("running"),
    },
    HookEvent {
        name: "Stop",
        matcher: None,
        status: Some("idle"),
    },
    HookEvent {
        name: "Notification",
        matcher: Some("permission_prompt|elicitation_dialog"),
        status: Some("waiting"),
    },
    HookEvent {
        name: "ElicitationResult",
        matcher: None,
        status: Some("running"),
    },
];

pub const AGENTS: &[AgentDef] = &[
    AgentDef {
        name: "claude",
        binary: "claude",
        aliases: &[],
        detection: DetectionMethod::Which("claude"),
        yolo: Some(YoloMode::CliFlag("--dangerously-skip-permissions")),
        instruction_flag: Some("--append-system-prompt {}"),
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_claude_status,
        container_env: &[("CLAUDE_CONFIG_DIR", "/root/.claude")],
        hook_config: Some(AgentHookConfig {
            settings_rel_path: ".claude/settings.json",
            events: CLAUDE_CURSOR_HOOK_EVENTS,
        }),
        resume: Some(ResumeConfig {
            exit_sequence: &[&["C-c"], &["C-c"]],
            resume_pattern: r"claude --resume\s+([0-9a-f-]+)",
            resume_flag: "--resume {}",
            timeout_secs: 10,
        }),
        fork_template: Some("--resume {} --fork-session"),
        session_id_flag: Some("--session-id {}"),
        sets_own_title: true,
        exact_session_runtime: None,
        supports_cross_agent_team: true,
    },
    AgentDef {
        name: "codex",
        binary: "codex",
        aliases: &[],
        detection: DetectionMethod::Which("codex"),
        yolo: Some(YoloMode::CliFlag(
            "--dangerously-bypass-approvals-and-sandbox",
        )),
        instruction_flag: Some("--config developer_instructions={}"),
        // Suppress the interactive startup update menu (gated upstream by
        // `check_for_update_on_startup` in codex-rs/tui/src/updates.rs): in a
        // managed pane any Enter reaching that menu selects "Update now" and
        // kills the agent.
        fixed_args: &["--config", "check_for_update_on_startup=false"],
        set_default_command: true,
        supports_host_launch: true,
        detect_status: status_detection::detect_codex_status,
        container_env: &[],
        // No hooks: see the `AgentHookConfig` doc. Codex panes are tracked
        // from its rollout files and its status from content detection.
        hook_config: None,
        resume: Some(ResumeConfig {
            exit_sequence: &[&["C-c"], &["C-c"]],
            resume_pattern: r"codex resume\s+([0-9a-f-]+)",
            resume_flag: "resume {}",
            timeout_secs: 10,
        }),
        fork_template: Some("fork {}"),
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: true,
    },
    AgentDef {
        name: "opencode",
        binary: "opencode",
        aliases: &["open-code"],
        detection: DetectionMethod::Which("opencode"),
        yolo: Some(YoloMode::EnvVar("OPENCODE_PERMISSION", r#"{"*":"allow"}"#)),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_opencode_status,
        container_env: &[],
        hook_config: None,
        resume: Some(ResumeConfig {
            exit_sequence: &[&["C-c"], &["C-c"]],
            resume_pattern: r"(ses_[A-Za-z0-9_-]+)",
            resume_flag: "--session {}",
            timeout_secs: 10,
        }),
        fork_template: Some("--session {} --fork"),
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: Some(ExactSessionRuntime::OwnedServer),
        supports_cross_agent_team: true,
    },
    AgentDef {
        name: "kimi",
        binary: "kimi",
        aliases: &["kimi-code"],
        detection: DetectionMethod::Which("kimi"),
        yolo: Some(YoloMode::CliFlag("--yolo")),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_kimi_status,
        container_env: &[],
        hook_config: None,
        resume: Some(ResumeConfig {
            exit_sequence: &[&["C-c"], &["C-c"]],
            resume_pattern: r"(session_[0-9a-fA-F-]+)",
            resume_flag: "--session {}",
            timeout_secs: 10,
        }),
        // Forking is the agent's own operation and kimi exposes none; a fork of
        // an exact session on a shared server would also need a second identity
        // AoE never minted.
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: Some(ExactSessionRuntime::SharedServer),
        supports_cross_agent_team: true,
    },
    AgentDef {
        name: "shell",
        binary: "shell",
        aliases: &["terminal"],
        detection: DetectionMethod::Which("sh"),
        yolo: None,
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_terminal_status,
        container_env: &[],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: false,
    },
    AgentDef {
        name: "vibe",
        binary: "vibe",
        aliases: &["mistral-vibe"],
        detection: DetectionMethod::RunWithArg("vibe", "--version"),
        yolo: Some(YoloMode::CliFlag("--agent auto-approve")),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_vibe_status,
        container_env: &[],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: false,
    },
    AgentDef {
        name: "cursor",
        binary: "agent",
        aliases: &["agent"],
        detection: DetectionMethod::Which("agent"),
        yolo: Some(YoloMode::CliFlag("--yolo")),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_cursor_status,
        container_env: &[("CURSOR_CONFIG_DIR", "/root/.cursor")],
        hook_config: Some(AgentHookConfig {
            settings_rel_path: ".cursor/settings.json",
            events: CLAUDE_CURSOR_HOOK_EVENTS,
        }),
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: false,
    },
    AgentDef {
        name: "copilot",
        binary: "copilot",
        aliases: &["github-copilot"],
        detection: DetectionMethod::Which("copilot"),
        yolo: Some(YoloMode::CliFlag("--yolo")),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_copilot_status,
        container_env: &[("COPILOT_CONFIG_DIR", "/root/.copilot")],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: false,
    },
    AgentDef {
        name: "pi",
        binary: "pi",
        aliases: &[],
        detection: DetectionMethod::Which("pi"),
        // Pi runs in full YOLO mode by default (no approval gates), so no flag needed.
        yolo: Some(YoloMode::AlwaysYolo),
        instruction_flag: None,
        fixed_args: &[],
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_pi_status,
        container_env: &[("PI_CODING_AGENT_DIR", "/root/.pi/agent")],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
        exact_session_runtime: None,
        supports_cross_agent_team: false,
    },
];

/// Look up an agent by canonical name.
pub fn get_agent(name: &str) -> Option<&'static AgentDef> {
    AGENTS.iter().find(|a| a.name == name)
}

/// All canonical agent names in registry order.
pub fn agent_names() -> Vec<&'static str> {
    AGENTS.iter().map(|a| a.name).collect()
}

/// Given the name of a process running in a pane (tmux's
/// `#{pane_current_command}`), return the agent it identifies.
///
/// Narrower than [`resolve_tool_name`], which matches a launch command loosely
/// and defaults to Claude. A process name is a single token, so a substring
/// match there would read a wrapper script or a path component as an agent, and
/// no agent at all must stay `None` rather than becoming a default: the caller
/// acts on positive identification only.
///
/// Most agents do not name their process after themselves -- Claude reports its
/// version string -- so `None` means "no evidence", never "not an agent".
pub fn agent_from_process_name(process: &str) -> Option<&'static str> {
    let name = process.trim();
    let name = name.strip_prefix('-').unwrap_or(name);
    let base = name.rsplit('/').next().unwrap_or(name);
    AGENTS
        .iter()
        .find(|a| a.name == base || a.binary == base)
        .map(|a| a.name)
}

/// Given a command string (e.g. `"claude --resume xyz"` or `"open-code"`),
/// return the canonical agent name if one is recognised.
pub fn resolve_tool_name(cmd: &str) -> Option<&'static str> {
    let cmd_lower = cmd.to_lowercase();
    if cmd_lower.is_empty() {
        return Some("claude");
    }
    for agent in AGENTS {
        if cmd_lower.contains(agent.name) {
            return Some(agent.name);
        }
        for alias in agent.aliases {
            if cmd_lower.contains(alias) {
                return Some(agent.name);
            }
        }
    }
    None
}

/// Convert a tool name to a 1-based settings index (0 = Auto).
pub fn settings_index_from_name(name: Option<&str>) -> usize {
    match name {
        Some(n) => AGENTS
            .iter()
            .position(|a| a.name == n)
            .map(|i| i + 1)
            .unwrap_or(0),
        None => 0,
    }
}

/// Convert a 1-based settings index back to a tool name (0 = Auto/None).
pub fn name_from_settings_index(index: usize) -> Option<&'static str> {
    if index == 0 {
        None
    } else {
        AGENTS.get(index - 1).map(|a| a.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process name is the only thing a restart has to go on for a pane no
    /// slot describes, so what it must NOT claim matters as much as what it
    /// recognizes: a wrong answer relaunches the pane as the wrong agent.
    #[test]
    fn test_agent_from_process_name_identifies_only_exact_binaries() {
        assert_eq!(agent_from_process_name("codex"), Some("codex"));
        assert_eq!(
            agent_from_process_name("/usr/local/bin/codex"),
            Some("codex")
        );
        assert_eq!(agent_from_process_name("  codex\n"), Some("codex"));

        // A login shell, and the shells a pane sits in between agents.
        assert_eq!(agent_from_process_name("-zsh"), None);
        assert_eq!(agent_from_process_name("bash"), None);
        // What a real Claude reports for itself: its version, not its name.
        assert_eq!(agent_from_process_name("2.1.220"), None);
        // An interpreter is not the agent it happens to be running.
        assert_eq!(agent_from_process_name("node"), None);
        // Substring matches are what `resolve_tool_name` does; here they would
        // read a neighboring binary as an agent.
        assert_eq!(agent_from_process_name("codexify"), None);
        assert_eq!(agent_from_process_name(""), None);
    }

    #[test]
    fn test_get_agent_known() {
        assert_eq!(get_agent("claude").unwrap().binary, "claude");
        assert_eq!(get_agent("opencode").unwrap().binary, "opencode");
        assert_eq!(get_agent("vibe").unwrap().binary, "vibe");
        assert_eq!(get_agent("codex").unwrap().binary, "codex");
        assert_eq!(get_agent("shell").unwrap().binary, "shell");
        assert_eq!(get_agent("cursor").unwrap().binary, "agent");
        assert_eq!(get_agent("copilot").unwrap().binary, "copilot");
        assert_eq!(get_agent("pi").unwrap().binary, "pi");
        assert_eq!(get_agent("kimi").unwrap().binary, "kimi");
    }

    #[test]
    fn test_get_agent_unknown() {
        assert!(get_agent("unknown").is_none());
    }

    #[test]
    fn opencode_supports_host_launch_and_exact_session_resume() {
        let opencode = get_agent("opencode").unwrap();
        assert!(opencode.supports_host_launch);
        assert!(!opencode.set_default_command);
        assert_eq!(
            opencode.resume.as_ref().unwrap().resume_flag,
            "--session {}"
        );
    }

    #[test]
    fn test_agent_names() {
        let names = agent_names();
        assert_eq!(
            names,
            vec!["claude", "codex", "opencode", "kimi", "shell", "vibe", "cursor", "copilot", "pi"]
        );
    }

    #[test]
    fn test_resolve_tool_name() {
        assert_eq!(resolve_tool_name("claude"), Some("claude"));
        assert_eq!(resolve_tool_name("open-code"), Some("opencode"));
        assert_eq!(resolve_tool_name("mistral-vibe"), Some("vibe"));
        assert_eq!(resolve_tool_name("codex"), Some("codex"));
        assert_eq!(resolve_tool_name("shell"), Some("shell"));
        assert_eq!(resolve_tool_name("terminal"), Some("shell"));
        assert_eq!(resolve_tool_name("cursor"), Some("cursor"));
        assert_eq!(resolve_tool_name("github-copilot"), Some("copilot"));
        assert_eq!(resolve_tool_name("copilot"), Some("copilot"));
        assert_eq!(resolve_tool_name("pi"), Some("pi"));
        assert_eq!(resolve_tool_name("kimi"), Some("kimi"));
        assert_eq!(resolve_tool_name("kimi-code"), Some("kimi"));
        assert_eq!(resolve_tool_name(""), Some("claude"));
        assert_eq!(resolve_tool_name("agent"), Some("cursor"));
        assert_eq!(resolve_tool_name("unknown-tool"), None);
    }

    #[test]
    fn test_settings_index_roundtrip() {
        assert_eq!(settings_index_from_name(None), 0);
        assert_eq!(settings_index_from_name(Some("claude")), 1);
        assert_eq!(settings_index_from_name(Some("codex")), 2);
        assert_eq!(settings_index_from_name(Some("opencode")), 3);
        assert_eq!(settings_index_from_name(Some("kimi")), 4);
        assert_eq!(settings_index_from_name(Some("shell")), 5);
        assert_eq!(settings_index_from_name(Some("vibe")), 6);
        assert_eq!(settings_index_from_name(Some("cursor")), 7);
        assert_eq!(settings_index_from_name(Some("copilot")), 8);
        assert_eq!(settings_index_from_name(Some("pi")), 9);

        assert_eq!(name_from_settings_index(0), None);
        assert_eq!(name_from_settings_index(1), Some("claude"));
        assert_eq!(name_from_settings_index(2), Some("codex"));
        assert_eq!(name_from_settings_index(3), Some("opencode"));
        assert_eq!(name_from_settings_index(4), Some("kimi"));
        assert_eq!(name_from_settings_index(5), Some("shell"));
        assert_eq!(name_from_settings_index(6), Some("vibe"));
        assert_eq!(name_from_settings_index(7), Some("cursor"));
        assert_eq!(name_from_settings_index(8), Some("copilot"));
        assert_eq!(name_from_settings_index(9), Some("pi"));
        assert_eq!(name_from_settings_index(99), None);
    }

    /// The registry is the only place an agent's capabilities are stated, so a
    /// launch path can branch on a field instead of recognizing a tool name.
    #[test]
    fn capabilities_are_registry_fields_rather_than_tool_names() {
        assert_eq!(
            exact_session_runtime("opencode"),
            Some(ExactSessionRuntime::OwnedServer)
        );
        assert_eq!(
            exact_session_runtime("kimi"),
            Some(ExactSessionRuntime::SharedServer)
        );
        for tool in ["claude", "codex", "vibe", "shell", "unknown-tool"] {
            assert_eq!(exact_session_runtime(tool), None, "{tool}");
        }

        for tool in ["claude", "codex", "opencode", "kimi"] {
            assert!(supports_cross_agent_team(tool), "{tool}");
        }
        for tool in ["vibe", "shell", "cursor", "copilot", "pi", "nope"] {
            assert!(!supports_cross_agent_team(tool), "{tool}");
        }
    }

    /// The one derived rule: AoE prepares an exact-session agent's conversation
    /// and holds its key, so the key never travels in that pane's environment.
    /// An agent AoE does not prepare carries its key the way it always did, and
    /// an unrecognized tool is launched undecorated.
    #[test]
    fn only_agents_aoe_prepares_keep_the_identity_key_out_of_the_pane() {
        for agent in AGENTS {
            assert_eq!(
                agent.identity_key_in_pane_env(),
                !agent.uses_exact_session_runtime(),
                "{}",
                agent.name
            );
        }
        assert!(!identity_key_in_pane_env("opencode"));
        assert!(!identity_key_in_pane_env("kimi"));
        assert!(identity_key_in_pane_env("claude"));
        assert!(identity_key_in_pane_env("unknown-tool"));
    }

    #[test]
    fn kimi_launches_on_the_host_and_resumes_an_exact_session() {
        let kimi = get_agent("kimi").unwrap();
        assert!(kimi.supports_host_launch);
        assert!(!kimi.set_default_command);
        assert_eq!(kimi.resume.as_ref().unwrap().resume_flag, "--session {}");
        // No pre-allocated id and no fork: the session is minted over REST
        // before launch, and the shared server offers no fork of it.
        assert_eq!(kimi.session_id_flag, None);
        assert_eq!(kimi.fork_template, None);
    }

    /// Only codex needs launch hardening today: its startup update menu blocks
    /// the pane and turns any Enter into an agent-killing update. Every other
    /// agent must stay undecorated so their commands are unaffected.
    #[test]
    fn only_codex_carries_fixed_args() {
        for agent in AGENTS {
            if agent.name == "codex" {
                assert_eq!(
                    agent.fixed_args,
                    &["--config", "check_for_update_on_startup=false"],
                    "codex must suppress the startup update check"
                );
            } else {
                assert!(
                    agent.fixed_args.is_empty(),
                    "agent '{}' must not carry fixed args",
                    agent.name
                );
            }
        }
    }

    #[test]
    fn test_all_agents_have_yolo_support() {
        for agent in AGENTS {
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
}
