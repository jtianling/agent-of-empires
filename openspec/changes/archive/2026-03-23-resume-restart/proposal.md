## Why

When pressing R to restart an agent pane (e.g., to reload skills), the current implementation kills the process and starts a completely fresh session.  This loses the entire conversation context.  Users currently must manually exit, copy a resume code, and restart with `--resume <code>` to preserve context.  This should be automated.

## What Changes

- Add a `ResumeConfig` field to `AgentDef` that declares how an agent supports graceful exit and session resumption
- Modify the R-key restart flow to attempt graceful exit first: send exit keys, capture pane output, parse a resume token, and restart with the token
- Implement a tick-driven state machine (`PendingResume`) so the TUI remains responsive during the graceful exit wait
- Add a new `Restarting` status variant for UI feedback
- Fall back to the current kill-and-fresh-start behavior when: the agent has no `ResumeConfig`, the instance uses a custom command, the graceful exit times out, or the resume token cannot be parsed
- Configure `ResumeConfig` for Claude (`--resume <uuid>`) and Codex (`resume <uuid>`)

## Capabilities

### New Capabilities
- `agent-resume-restart`: Graceful agent pane restart that preserves conversation context by capturing a resume token from the exiting agent and passing it to the new process

### Modified Capabilities
- `agent-pane-restart`: R-key restart now defaults to graceful resume flow instead of immediate kill, with fallback to current behavior
- `agent-registry`: `AgentDef` gains a new `resume: Option<ResumeConfig>` field

## Impact

- `src/agents.rs`: new `ResumeConfig` struct, new field on `AgentDef`, config for Claude and Codex
- `src/session/instance.rs`: `PendingResume` state, modified `respawn_agent_pane()`, `build_agent_command()` gains resume token parameter
- `src/session/mod.rs`: new `Restarting` status variant
- `src/tmux/session.rs`: new `send_keys_to_agent_pane()` method
- `src/tui/app.rs`: `RespawnAgentPane` action initiates graceful flow, tick handler drives state machine
- Status bar and status detection may need to handle the new `Restarting` status
