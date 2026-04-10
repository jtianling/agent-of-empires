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
pub struct AgentHookConfig {
    /// Path relative to the home dir where the agent's settings live
    /// (e.g. `.claude/settings.json`).
    pub settings_rel_path: &'static str,
    /// Hook events to register (status transitions).
    pub events: &'static [HookEvent],
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
    },
    AgentDef {
        name: "opencode",
        binary: "opencode",
        aliases: &["open-code"],
        detection: DetectionMethod::Which("opencode"),
        yolo: Some(YoloMode::EnvVar("OPENCODE_PERMISSION", r#"{"*":"allow"}"#)),
        instruction_flag: None,
        set_default_command: true,
        supports_host_launch: false,
        detect_status: status_detection::detect_opencode_status,
        container_env: &[],
        hook_config: None,
        resume: None,
        fork_template: Some("--session {} --fork"),
        session_id_flag: None,
        sets_own_title: false,
    },
    AgentDef {
        name: "vibe",
        binary: "vibe",
        aliases: &["mistral-vibe"],
        detection: DetectionMethod::RunWithArg("vibe", "--version"),
        yolo: Some(YoloMode::CliFlag("--agent auto-approve")),
        instruction_flag: None,
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_vibe_status,
        container_env: &[],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
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
        set_default_command: true,
        supports_host_launch: true,
        detect_status: status_detection::detect_codex_status,
        container_env: &[],
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
    },
    AgentDef {
        name: "gemini",
        binary: "gemini",
        aliases: &[],
        detection: DetectionMethod::Which("gemini"),
        yolo: Some(YoloMode::CliFlag("--approval-mode yolo")),
        instruction_flag: None,
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_gemini_status,
        container_env: &[],
        hook_config: Some(AgentHookConfig {
            settings_rel_path: ".gemini/settings.json",
            events: &[
                HookEvent {
                    name: "BeforeTool",
                    matcher: None,
                    status: Some("running"),
                },
                HookEvent {
                    name: "BeforeAgent",
                    matcher: None,
                    status: Some("running"),
                },
                HookEvent {
                    name: "AfterAgent",
                    matcher: None,
                    status: Some("idle"),
                },
                HookEvent {
                    name: "Notification",
                    matcher: Some("ToolPermission"),
                    status: Some("waiting"),
                },
            ],
        }),
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: true,
    },
    AgentDef {
        name: "shell",
        binary: "shell",
        aliases: &["terminal"],
        detection: DetectionMethod::Which("sh"),
        yolo: None,
        instruction_flag: None,
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_terminal_status,
        container_env: &[],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
    },
    AgentDef {
        name: "cursor",
        binary: "agent",
        aliases: &["agent"],
        detection: DetectionMethod::Which("agent"),
        yolo: Some(YoloMode::CliFlag("--yolo")),
        instruction_flag: None,
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
    },
    AgentDef {
        name: "copilot",
        binary: "copilot",
        aliases: &["github-copilot"],
        detection: DetectionMethod::Which("copilot"),
        yolo: Some(YoloMode::CliFlag("--yolo")),
        instruction_flag: None,
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_copilot_status,
        container_env: &[("COPILOT_CONFIG_DIR", "/root/.copilot")],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
    },
    AgentDef {
        name: "pi",
        binary: "pi",
        aliases: &[],
        detection: DetectionMethod::Which("pi"),
        // Pi runs in full YOLO mode by default (no approval gates), so no flag needed.
        yolo: Some(YoloMode::AlwaysYolo),
        instruction_flag: None,
        set_default_command: false,
        supports_host_launch: true,
        detect_status: status_detection::detect_pi_status,
        container_env: &[("PI_CODING_AGENT_DIR", "/root/.pi/agent")],
        hook_config: None,
        resume: None,
        fork_template: None,
        session_id_flag: None,
        sets_own_title: false,
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

    #[test]
    fn test_get_agent_known() {
        assert_eq!(get_agent("claude").unwrap().binary, "claude");
        assert_eq!(get_agent("opencode").unwrap().binary, "opencode");
        assert_eq!(get_agent("vibe").unwrap().binary, "vibe");
        assert_eq!(get_agent("codex").unwrap().binary, "codex");
        assert_eq!(get_agent("gemini").unwrap().binary, "gemini");
        assert_eq!(get_agent("shell").unwrap().binary, "shell");
        assert_eq!(get_agent("cursor").unwrap().binary, "agent");
        assert_eq!(get_agent("copilot").unwrap().binary, "copilot");
        assert_eq!(get_agent("pi").unwrap().binary, "pi");
    }

    #[test]
    fn test_get_agent_unknown() {
        assert!(get_agent("unknown").is_none());
    }

    #[test]
    fn test_agent_names() {
        let names = agent_names();
        assert_eq!(
            names,
            vec![
                "claude", "opencode", "vibe", "codex", "gemini", "shell", "cursor", "copilot", "pi"
            ]
        );
    }

    #[test]
    fn test_resolve_tool_name() {
        assert_eq!(resolve_tool_name("claude"), Some("claude"));
        assert_eq!(resolve_tool_name("open-code"), Some("opencode"));
        assert_eq!(resolve_tool_name("mistral-vibe"), Some("vibe"));
        assert_eq!(resolve_tool_name("codex"), Some("codex"));
        assert_eq!(resolve_tool_name("gemini"), Some("gemini"));
        assert_eq!(resolve_tool_name("shell"), Some("shell"));
        assert_eq!(resolve_tool_name("terminal"), Some("shell"));
        assert_eq!(resolve_tool_name("cursor"), Some("cursor"));
        assert_eq!(resolve_tool_name("github-copilot"), Some("copilot"));
        assert_eq!(resolve_tool_name("copilot"), Some("copilot"));
        assert_eq!(resolve_tool_name("pi"), Some("pi"));
        assert_eq!(resolve_tool_name(""), Some("claude"));
        assert_eq!(resolve_tool_name("agent"), Some("cursor"));
        assert_eq!(resolve_tool_name("unknown-tool"), None);
    }

    #[test]
    fn test_settings_index_roundtrip() {
        assert_eq!(settings_index_from_name(None), 0);
        assert_eq!(settings_index_from_name(Some("claude")), 1);
        assert_eq!(settings_index_from_name(Some("gemini")), 5);
        assert_eq!(settings_index_from_name(Some("shell")), 6);
        assert_eq!(settings_index_from_name(Some("cursor")), 7);
        assert_eq!(settings_index_from_name(Some("copilot")), 8);
        assert_eq!(settings_index_from_name(Some("pi")), 9);

        assert_eq!(name_from_settings_index(0), None);
        assert_eq!(name_from_settings_index(1), Some("claude"));
        assert_eq!(name_from_settings_index(5), Some("gemini"));
        assert_eq!(name_from_settings_index(6), Some("shell"));
        assert_eq!(name_from_settings_index(7), Some("cursor"));
        assert_eq!(name_from_settings_index(8), Some("copilot"));
        assert_eq!(name_from_settings_index(9), Some("pi"));
        assert_eq!(name_from_settings_index(99), None);
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
