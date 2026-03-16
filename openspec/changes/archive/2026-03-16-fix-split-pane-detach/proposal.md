## Why

When a user creates tmux split panes (Ctrl+b % or Ctrl+b ") inside an AoE-managed session, detaching from a user-created pane causes AoE to kill and recreate the entire session, destroying all splits. This happens because AoE's pane health checks (`is_pane_dead`, `is_pane_running_shell`) target the session's currently active pane rather than the original agent pane. If the active pane is a user-created shell, AoE misinterprets it as the agent having crashed.

## What Changes

- Store the original agent pane ID (`#{pane_id}`) as a tmux session-level option (`@aoe_agent_pane`) when creating a session
- Change all pane health check functions (`is_pane_dead`, `is_pane_running_shell`, `get_pane_pid`, `pane_current_command`) to accept an explicit pane target parameter
- Update all call sites to target the stored agent pane ID instead of the session's current active pane
- Apply the same fix to all session types: `Session`, `TerminalSession`, `ContainerTerminalSession`

## Capabilities

### New Capabilities

_None_

### Modified Capabilities

- `session-management`: Pane health checks must target the original agent pane, not the session's active pane. Session attach must tolerate user-created split panes without killing the session.

## Impact

- `src/tmux/utils.rs`: `is_pane_dead()`, `is_pane_running_shell()`, `pane_current_command()`, `append_remain_on_exit_args()` gain pane-aware targeting
- `src/tmux/session.rs`: `create_with_size()` stores agent pane ID; `is_pane_dead()`, `is_pane_running_shell()`, `get_pane_pid()` use stored pane ID
- `src/tmux/terminal_session.rs`: Same changes for `TerminalSession` and `ContainerTerminalSession`
- `src/tui/app.rs`: Attach flow benefits from fixed checks (no code change needed if session methods are updated)
- `src/session/status_poller.rs`: Status polling benefits from fixed checks
