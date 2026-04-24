## Why

Users often create a `shell` session in AoE and then manually launch an agent (claude, codex, gemini, etc.) inside that shell. The TUI status indicator stays at `?` (Unknown) for the lifetime of that pane because the shell tool's status detector is a stub that always returns `Idle`, and no runtime discovery is attempted on the primary pane. As a result, users cannot see when the manually-launched agent is running, waiting, or idle — losing the main value of the status bar for a common workflow.

## What Changes

- On detach from a shell session back to the AoE TUI, run a one-shot agent detection against the primary pane (reusing `detect_agent_type_from_pane`).
- Store the detected agent in an in-memory field on the session (not persisted to disk). The session's `tool` field remains `shell`.
- During status polling, when `tool == "shell"` AND `detected_agent == Some(X)`, dispatch to agent `X`'s content-based detector instead of the shell stub. Fall back to current `?` behavior when detection returns `None` or the detected agent has no content-based detector.
- Re-run detection on every subsequent detach (cheap, bounded to one detection per detach cycle). Previous `detected_agent` is overwritten or cleared.
- No changes for agent-type sessions — they continue to use their existing detectors and hooks.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `status-detection`: add a shell-inner-agent discovery path that overrides the shell stub when an agent is detected in the primary pane after a detach event.

## Impact

- `src/session/instance.rs`: new in-memory field `detected_inner_agent: Option<String>` on the session; status update path reads this before dispatching.
- `src/tmux/status_detection.rs`: make `detect_agent_type_from_pane` (or equivalent) usable against the primary pane, not only extra panes.
- `src/tui/app.rs` / `src/cli/session.rs`: attach-return path triggers the one-shot detection.
- `src/agents.rs`: no change to `sets_own_title`, no change to hook_config.
- No persisted data changes, no migration required.
- No breaking changes.
