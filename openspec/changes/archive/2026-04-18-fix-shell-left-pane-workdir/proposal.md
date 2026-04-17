## Why

When creating a new session with Shell as the left pane tool, the shell starts in the AoE launch directory instead of the path specified in the new session dialog. The right pane shell already handles this correctly with an explicit `cd`, but the left pane relies solely on tmux's `-c` flag which can be overridden by login shell profile scripts (`.zprofile`, `.zlogin`).

## What Changes

- Add an explicit `cd` to the left pane command for shell sessions, matching the pattern already used by the right pane shell in `build_right_pane_command`
- The fix is scoped to the `build_agent_command` method in `src/session/instance.rs`, specifically the non-sandboxed shell path

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `session-management`: Shell sessions on the left pane must start in the user-specified `project_path`, not the AoE launch directory

## Impact

- **Code**: `src/session/instance.rs` (`build_agent_command` or `wrap_command_ignore_suspend_with_env`)
- **Behavior**: Shell left-pane sessions will now reliably start in the correct working directory
- **Risk**: Low. The change mirrors an existing proven pattern from the right pane code path
