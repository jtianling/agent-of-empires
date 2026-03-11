## Context

AoE manages AI coding agent sessions via tmux. The agent registry (`src/agents.rs`) defines all supported tools as `AgentDef` entries. The TUI new session dialog iterates `AvailableTools` (detected at startup) to populate the tool picker. Terminal sessions already exist as a paired feature (`TerminalSession` in `src/tmux/terminal_session.rs`) but are not independently creatable from the dialog -- they can only be opened alongside an existing agent session.

## Goals / Non-Goals

**Goals:**
- Add "terminal" as a first-class tool in the agent registry, positioned after gemini and before cursor.
- Terminal is always available (no binary detection).
- Creating a "terminal" session launches the user's default shell in the specified working directory.
- Agent-specific UI fields (YOLO mode, worktree/branch) are hidden when terminal is selected.

**Non-Goals:**
- Custom shell selection (users can set this via command override if needed).
- Terminal-specific status detection (shell prompt parsing is unreliable and not useful).
- Container/sandbox support for terminal sessions (terminal is host-only for now).

## Decisions

### D1: Terminal as an AgentDef entry

Add terminal as a regular `AgentDef` in the `AGENTS` array. This is the simplest approach -- it reuses all existing infrastructure (tool picker, settings index, name resolution) without special-casing.

Alternative considered: A separate "category" enum wrapping agents vs terminals. Rejected because it would require changes across the entire codebase (tool picker, session storage, preview, settings) for minimal benefit.

**Key field values:**
- `name: "terminal"`, `binary: "terminal"` (not actually invoked as a binary)
- `detection: DetectionMethod::Which("sh")` -- sh is always present on Unix
- `yolo: None` -- no auto-approve concept for a shell
- `instruction_flag: None`
- `set_default_command: false` -- the session builder will need to handle terminal specially (launch shell, not a binary)
- `supports_host_launch: true`
- `detect_status: detect_terminal_status` -- stub returning Idle
- `container_env: &[]`, `hook_config: None`

### D2: Session creation for terminal

When the tool is "terminal", the session builder should launch the user's `$SHELL` (or fall back to `/bin/sh`) in the specified working directory. The existing `TerminalSession` tmux prefix (`aoe_term_`) can be used, but since we're going through the normal session creation flow, we'll use the standard `aoe_` prefix so terminal sessions appear in the main session list alongside agents.

### D3: Hiding irrelevant fields

When terminal is selected in the new session dialog, the following fields are hidden:
- YOLO Mode (no concept of auto-approve)
- Worktree / Branch (terminal doesn't need git worktree isolation)

Fields that remain:
- Profile, Title, Path, Group, Sandbox, Image, Environment (all still useful for terminal sessions)

### D4: Position in registry

Terminal is inserted after gemini (index 5) and before cursor (index 6) in the `AGENTS` array. This matches the user's request.

## Risks / Trade-offs

- **[Risk] FR-001 violation**: The existing spec requires all agents to have YOLO mode. Terminal has `yolo: None`.
  -> Mitigation: Update FR-001 to allow `None` for non-agent tools, or make the test conditional.

- **[Risk] Settings index shift**: Adding terminal between gemini and cursor shifts cursor's settings index from 6 to 7.
  -> Mitigation: This is a one-time change. Users who had `default_tool = "cursor"` use the string name, not the index, so no breakage.

- **[Risk] `resolve_tool_name("")` still returns "claude"**: Terminal won't be the default.
  -> No mitigation needed -- this is correct behavior.
