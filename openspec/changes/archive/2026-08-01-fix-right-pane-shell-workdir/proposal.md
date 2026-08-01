## Why

When creating a new session with a shell right pane, the shell starts in AoE's launch directory instead of the session's configured working directory. This violates the existing right-pane spec requirement (line 30: "the right pane SHALL use the same working directory as the main session") and creates a confusing UX where the user must manually `cd` to the correct directory after every new session creation with a shell right pane.

## What Changes

- Fix the shell right pane to use the session's `project_path` as its working directory, matching the left pane
- Add diagnostic logging to `split_window_right` to aid future debugging of working directory issues
- Add an e2e test verifying right pane shell working directory matches the session path

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `right-pane`: The right pane shell working directory must match the session's project_path, not AoE's launch directory. This is already a stated requirement but the implementation has a bug.

## Impact

- `src/tui/app.rs`: `build_right_pane_command()` and the right pane split logic in `attach_session()`
- `src/tmux/session.rs`: `split_window_right()` -- may need to verify `-c` flag handling
- `tests/e2e/`: new test to verify working directory behavior
