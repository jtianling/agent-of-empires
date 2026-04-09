## Context

When creating a new session with a right pane set to "shell", the right pane's shell starts in AoE's launch directory instead of the session's configured `project_path`. The current code path in `src/tui/app.rs:698-708` passes `inst.project_path` to `split_window_right()`, and `split_window_right()` in `src/tmux/session.rs:416-446` passes it via tmux's `-c` flag. Despite this, the shell opens in the wrong directory.

Key files:
- `src/tui/app.rs`: `build_right_pane_command()` (line 56) and right pane split in `attach_session()` (line 698)
- `src/tmux/session.rs`: `split_window_right()` (line 416)
- `openspec/specs/right-pane/spec.md`: existing spec requiring same working directory (line 30)

## Goals / Non-Goals

**Goals:**
- Shell right pane starts in the same working directory as the left (agent) pane
- Non-shell right pane tools also use the correct working directory (verify)
- Add test coverage for right pane working directory

**Non-Goals:**
- Changing the right pane feature's overall architecture
- Modifying how sandboxed right panes resolve container working directories

## Decisions

### 1. Root cause investigation approach

The `-c` flag is passed to tmux but the shell still starts in the wrong directory. Possible causes:

- **tmux command argument ordering**: The command string `bash -c 'stty susp undef; exec $SHELL'` is passed as a single arg to `split-window`. tmux may interpret this differently depending on how it parses the command -- it might execute it via `$SHELL -c <cmd>` which could reset the working directory if the login shell sources a profile that changes directories.
- **The `exec` in the wrapper**: `bash -c 'stty susp undef; exec /bin/zsh'` replaces bash with zsh. If zsh starts as a login shell (due to how tmux invokes it), it may source `.zprofile` which could `cd` elsewhere. But this would be user-specific and unlikely to be the general bug.
- **Race or ordering issue**: The split might happen before the tmux session's default directory is fully set.

**Decision**: Investigate and fix the actual root cause. Start by adding debug logging to `split_window_right` to capture the exact tmux args, then verify the tmux command works correctly in isolation. If the `-c` flag is confirmed working, look for shell initialization overrides.

### 2. Fix strategy

Regardless of root cause, ensure the right pane shell starts in the correct directory by:
1. Verifying `split_window_right()` passes `-c` correctly
2. If the issue is with tmux's handling of `-c` combined with shell commands, consider an alternative approach: prepend `cd <dir> &&` to the shell command as a defense-in-depth measure
3. Add a debug log line showing the exact working directory and command being used

### 3. Test approach

Add an e2e test that:
1. Creates a session with a shell right pane and a specific project path (temp dir)
2. Captures the right pane's output after running `pwd`
3. Verifies it matches the session's project path

## Risks / Trade-offs

- [Minimal risk] Adding `cd <dir>` as defense-in-depth is redundant if `-c` works, but harmless and provides an extra safety net.
- [Low risk] The e2e test depends on tmux being available (existing tests already auto-skip without tmux).
