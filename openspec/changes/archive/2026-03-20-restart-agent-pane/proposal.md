## Why

When a code agent (e.g., Claude Code) exits in an AoE-managed session, the pane shows "Pane is dead" due to `remain-on-exit`. The only way to restart it is to detach back to the TUI and re-attach, which triggers `kill-session` and recreates the session from scratch. This destroys any custom layout and additional panes the user manually created (via `Ctrl+b %` / `Ctrl+b "`). Users frequently build complex multi-pane layouts within an AoE session and need a way to restart just the agent pane without losing their work.

## What Changes

- Add `R` (Shift+R) keybinding in the TUI home screen to restart only the agent pane of the selected session, using `tmux respawn-pane` instead of `kill-session` + recreate.
- Modify the attach-time restart logic: when the agent pane is dead but the session has user-created panes, use `respawn-pane` to preserve the layout instead of destroying the entire session.
- Extract the agent launch command construction into a reusable function so both `start_with_size_opts()` and the new respawn path can share it.

## Capabilities

### New Capabilities

- `agent-pane-restart`: Restart only the AoE-managed agent pane within a session, preserving the tmux session layout and any user-created panes.

### Modified Capabilities

- `session-management`: Attach-time restart logic should prefer `respawn-pane` over `kill-session` when the session has multiple panes.
- `tui`: Add `R` keybinding to the home screen for agent-pane-only restart.
- `status-detection`: After respawn, status should transition to `Starting` and resume normal detection.

## Impact

- `src/tmux/session.rs`: Add `respawn_agent_pane()` method.
- `src/tmux/utils.rs`: Process tree cleanup scoped to single pane.
- `src/session/instance.rs`: Extract command construction; add `respawn_agent_pane()` method.
- `src/tui/app.rs`: Attach-time logic to prefer respawn over kill when multi-pane.
- `src/tui/home.rs` or input handler: Wire `R` keybinding.
