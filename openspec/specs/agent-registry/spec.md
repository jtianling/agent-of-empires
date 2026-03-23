# Capability Spec: Agent Registry

**Capability**: `agent-registry`
**Created**: 2026-03-06
**Status**: Stable

## Overview

The Agent Registry is a centralized static table (`AGENTS` in `src/agents.rs`) that declares
every supported AI coding agent. All per-agent behavior -- binary name, status detection,
YOLO mode, instruction injection, host launch support -- is defined here. Adding a new agent
requires only adding one `AgentDef` entry and writing a status detection function.

## Key Entities

### AgentDef

```rust
pub struct AgentDef {
    pub name: &'static str,           // canonical name: "claude", "codex", etc.
    pub binary: &'static str,         // executable to invoke
    pub aliases: &'static [&'static str],  // alternative name substrings
    pub detection: DetectionMethod,   // how to check if installed
    pub yolo: Option<YoloMode>,       // auto-approve mode
    pub instruction_flag: Option<&'static str>,  // CLI flag for custom instruction ({} = text)
    pub set_default_command: bool,    // set instance.command = binary by default
    pub supports_host_launch: bool,   // can run directly on host (not just in container)
    pub detect_status: fn(&str) -> Status,  // parse pane content -> Status
    pub container_env: &'static [(&'static str, &'static str)],  // always-injected env vars
    pub hook_config: Option<HookConfig>,  // optional hook configuration
    pub sets_own_title: bool,  // agent sets terminal/pane title via OSC 0
    pub resume: Option<ResumeConfig>,  // optional resume configuration for graceful restart
}
```

### DetectionMethod

```
Which(binary)        -- run `which <binary>`, check exit code
RunWithArg(bin, arg) -- run `<bin> <arg>` (e.g. `vibe --version`), check it doesn't error
```

### YoloMode

```
CliFlag(flag)        -- append flag to command (e.g. "--dangerously-skip-permissions")
EnvVar(key, value)   -- prepend env var to command (e.g. "OPENCODE_PERMISSION=...")
```

## Registered Agents

| Name | Binary | YOLO | Instruction Flag | Host Launch | Own Title | Resume | Notes |
|------|--------|------|-----------------|-------------|-----------|--------|-------|
| `claude` | `claude` | `--dangerously-skip-permissions` | `--append-system-prompt {}` | Yes | Yes | Yes | Default agent |
| `opencode` | `opencode` | `OPENCODE_PERMISSION={"*":"allow"}` | None | No | No | No | Container-only |
| `vibe` | `vibe` | `--agent auto-approve` | None | Yes | No | No | Detection: `vibe --version` |
| `codex` | `codex` | `--dangerously-bypass-approvals-and-sandbox` | `--config developer_instructions={}` | Yes | No | Yes | |
| `gemini` | `gemini` | `--approval-mode yolo` | None | Yes | Yes | No | |
| `shell` | `shell` | None | None | Yes | No | No | Plain shell, no status detection. Alias: `terminal` |
| `cursor` | `agent` | `--yolo` | None | Yes | No | No | Binary is `agent`, aliases: `["agent"]` |

## Name Resolution

Given a command string (e.g. `"claude --resume xyz"` or `"open-code"`):
1. Lowercase the command
2. Check if it contains any agent's `name` substring
3. Check if it contains any alias substring
4. Return canonical name or `None` if unrecognized
5. Empty command string resolves to `"claude"` (default)

## Container Environment

Agents may declare env vars that are always injected into container sessions:
- `claude`: `CLAUDE_CONFIG_DIR=/root/.claude`
- `cursor`: `CURSOR_CONFIG_DIR=/root/.cursor`

## Functional Requirements

- **FR-001**: All agents MUST have a `yolo` mode configured, except for non-agent tools (e.g., shell) where `yolo: None` is permitted.
- **FR-002**: Status detection functions MUST accept raw (non-lowercased) pane content.
- **FR-003**: Agent availability MUST be detected at runtime, not hardcoded.
- **FR-004**: Name resolution MUST be case-insensitive (command is lowercased before matching).
- **FR-005**: The `instruction_flag` template MUST use `{}` as the placeholder for the escaped instruction text.
- **FR-006**: Agents with `supports_host_launch: false` MUST only be launched inside containers.
- **FR-007**: Adding a new agent MUST NOT require changes outside `src/agents.rs` and `src/tmux/status_detection.rs`.
- **FR-008**: Agents with `sets_own_title: false` SHALL have their tmux pane title managed by AoE based on detected status. AoE SHALL prefix the title with a waiting icon when the agent's status is `Waiting`.
- **FR-009**: Agents with `sets_own_title: true` (claude, gemini) SHALL NOT have their pane title overwritten by AoE.

### Requirement: All agents MUST have a yolo mode configured
All agents MUST have a `yolo` mode configured, except for non-agent tools (e.g., shell) where `yolo: None` is permitted.

#### Scenario: Agent tools have YOLO support
- **WHEN** iterating over agent entries in the registry (excluding shell)
- **THEN** each entry has `yolo.is_some() == true`

#### Scenario: Shell tool has no YOLO
- **WHEN** querying the shell entry's YOLO mode
- **THEN** it returns `None`

### Requirement: Shell entry in registry
The agent registry SHALL include a `shell` entry with `name: "shell"`, positioned after `gemini` and before `cursor` in the `AGENTS` array. The alias `"terminal"` SHALL resolve to `"shell"`.

#### Scenario: Shell is registered
- **WHEN** looking up agent by name "shell"
- **THEN** an `AgentDef` is returned with `name == "shell"`

#### Scenario: Terminal alias resolves to shell
- **WHEN** resolving tool name "terminal"
- **THEN** the result is `"shell"`

#### Scenario: Registry order includes shell
- **WHEN** listing all agent names in registry order
- **THEN** the list is `["claude", "opencode", "vibe", "codex", "gemini", "shell", "cursor"]`

#### Scenario: Settings index accounts for shell
- **WHEN** converting "shell" to a settings index
- **THEN** the result is `6` (gemini=5, shell=6, cursor=7)

### Requirement: AgentDef supports optional resume configuration
`AgentDef` SHALL include a `resume: Option<ResumeConfig>` field. Agents that support session resumption declare their exit sequence, output pattern, and resume CLI flag via this field. Agents that do not support resume set this to `None`.

#### Scenario: Claude declares resume support
- **WHEN** the Claude agent definition is loaded
- **THEN** it SHALL have a `ResumeConfig` with:
  - exit sequence: two Ctrl+C key groups (one per tick)
  - resume pattern matching `claude --resume` followed by a UUID
  - resume flag template `--resume {}`
  - timeout of 10 seconds

#### Scenario: Codex declares resume support
- **WHEN** the Codex agent definition is loaded
- **THEN** it SHALL have a `ResumeConfig` with:
  - exit sequence: two Ctrl+C key groups (one per tick)
  - resume pattern matching `codex resume` followed by a UUID
  - resume flag template `resume {}`
  - timeout of 10 seconds

#### Scenario: Agents without resume support
- **WHEN** agent definitions for opencode, vibe, gemini, shell, or cursor are loaded
- **THEN** their `resume` field SHALL be `None`

### Requirement: ResumeConfig structure
`ResumeConfig` SHALL contain: an exit key sequence (array of key groups sent one group per tick), a regex pattern for capturing the resume token (first capture group), a flag template with `{}` placeholder for the token, and a timeout in seconds.

#### Scenario: ResumeConfig fields are complete
- **WHEN** a `ResumeConfig` is defined for an agent
- **THEN** it SHALL have all four fields: `exit_sequence`, `resume_pattern`, `resume_flag`, `timeout_secs`
- **AND** `resume_pattern` SHALL contain exactly one capture group for the token

## Settings Index Convention

The settings TUI uses a 1-based index for agent selection:
- `0` = Auto (detect first available)
- `1..N` = Agents in `AGENTS` registry order

`settings_index_from_name()` and `name_from_settings_index()` are the canonical converters.

## Success Criteria

- **SC-001**: All 7 currently registered agents have full behavior coverage (detection, YOLO, status).
- **SC-002**: A new agent can be added by modifying only the registry and status detection.
- **SC-003**: `resolve_tool_name("")` returns `"claude"` (default fallback).
- **SC-004**: Agent availability detection is fast enough to run at TUI startup without perceptible delay.
