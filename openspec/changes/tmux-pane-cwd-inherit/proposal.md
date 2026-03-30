## Why

When users manually split panes in AoE-managed tmux sessions (Ctrl+b % or Ctrl+b "), the new pane opens in `$HOME` or the AoE process's working directory instead of the session's project path. This is disorienting because users split panes to run auxiliary commands (git, tests, builds) alongside their agent and expect to already be in the project directory.

## What Changes

- Store `@aoe_project_path` as a tmux session option during session creation, recording the project's working directory for each managed session.
- Override the `%` and `"` prefix-table keybindings with if-shell guards that pass `-c '#{@aoe_project_path}'` to `split-window` when inside an AoE-managed session, falling back to default split behavior otherwise.
- Restore default `%` and `"` bindings on TUI exit cleanup.
- Backfill `@aoe_project_path` for existing sessions during the binding setup phase so older sessions gain the behavior without recreation.

## Capabilities

### New Capabilities
- `pane-cwd-inherit`: Ensures manually-split panes in AoE-managed tmux sessions inherit the session's project path as their working directory.

### Modified Capabilities
- `root-key-session-cycle`: The keybinding setup/cleanup functions gain two new bindings (% and ") with the same if-shell guard pattern used by existing session-cycle keys.

## Impact

- `src/tmux/session.rs` (`create_with_size`): Adds a `set-option` command to store `@aoe_project_path` alongside the existing `@aoe_agent_pane` store.
- `src/tmux/utils.rs` (`setup_session_cycle_bindings`, `cleanup_session_cycle_bindings`, `collect_tag_sessions_with_profile`): New bind-key lines for `%` and `"`, cleanup restores defaults, backfill sets `@aoe_project_path` for every known session.
- `tests/e2e/`: New e2e test verifying split-pane working directory.
- No new dependencies. No breaking changes. No config schema changes.
