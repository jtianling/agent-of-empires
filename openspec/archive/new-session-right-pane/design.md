## Context

AoE users frequently create dual-pane layouts by manually splitting tmux sessions (`Ctrl+b %`) after session creation, then launching a second tool in the new pane. This is a multi-step manual process. The new session dialog already captures tool selection; extending it with a "right pane" tool selector automates the split.

The existing `@aoe_agent_pane` mechanism (commit `ab54364`) stores the initial pane ID at session creation to ensure status detection and health checks always target the correct (left/agent) pane, even when users manually create splits. This mechanism is critical for correctness and must work correctly with the new automatic split.

## Goals / Non-Goals

**Goals:**
- Allow users to select a right pane tool in the new session dialog
- Automatically split the tmux session and launch the selected tool after session creation
- Maintain correct `@aoe_agent_pane` tracking so status detection targets the left (main) pane
- The right pane should behave identically to a manually created `Ctrl+b %` split

**Non-Goals:**
- Persisting right pane tool choice in Instance (it is a one-time creation action, like how `Ctrl+b %` is not tracked)
- Supporting more than two panes from the dialog
- Right pane configuration (extra_args, yolo_mode, etc.) -- only the tool binary is selected
- Restarting or managing the right pane lifecycle from AoE

## Decisions

### Decision 1: Right pane as a one-shot creation action, not persisted state

The right pane split is a one-time tmux operation during session creation. It is NOT stored in `Instance` or `NewSessionData`'s persisted fields. Rationale: this matches the mental model of "AoE does `Ctrl+b %` for me" -- once the pane exists, tmux manages it. AoE does not need to restart, track, or manage the right pane.

**Alternative considered**: Storing right pane tool in Instance for restart support. Rejected because it adds complexity for a rare case, and users who need restart can recreate the session.

### Decision 2: Split after session creation, using tmux `split-window -h`

After `Session::create_with_size()` completes and `@aoe_agent_pane` is set, execute `tmux split-window -h -t <session_name> -c <working_dir> <right_pane_command>`. This creates a vertical split (left/right) within the existing session.

The `-t` target uses the session name, which defaults to the first window. The split creates a new pane to the right of the existing agent pane.

**Alternative considered**: Using `new-window` instead of `split-window`. Rejected because the user wants side-by-side layout within the same window, not separate tabs.

### Decision 3: Right pane tool uses the same command construction as the main tool

The right pane tool command is built using the same `wrap_command_ignore_suspend` pattern as the main tool. It uses the tool binary with YOLO mode applied (both CliFlag and EnvVar variants), but no extra_args and no custom instruction. For "shell" tool, the user's `$SHELL` is used instead of the agent binary.

The right pane tool also gets `remain-on-exit on` set at the pane level, matching the main pane's behavior. This prevents the pane from disappearing if the tool exits.

### Decision 4: Dialog field placement and UX

The "Right Pane" field appears directly below the "Tool" field in the new session dialog and is always visible (including when the main tool is "shell"). It uses the same Left/Right arrow key cycling as the Tool field, but the options list starts with "none" followed by the same available tools. When "none" is selected (default), no split occurs.

The field is visible as a single line (no sub-configuration like Ctrl+P for tool config). The field index in `NewSessionDialog` shifts subsequent fields (YOLO, Worktree, Sandbox, etc.) down by one.

### Decision 5: `@aoe_agent_pane` correctness

The `@aoe_agent_pane` option is set during `Session::create_with_size()` as part of the atomic tmux command chain. The subsequent `split-window` command creates a new pane but does NOT modify `@aoe_agent_pane`. This means:

- Status detection (`detect_status`, `is_pane_dead`, `is_pane_running_shell`) continues to target the left pane
- Detaching from either pane and returning to AoE works correctly
- The right pane's process state does not affect session status in AoE

This is the same correctness guarantee that exists for manually created splits.

## Risks / Trade-offs

- **[Risk] Right pane tool not installed**: If the user selects a tool that isn't installed, the right pane will show an error. -> **Mitigation**: Same as the main tool -- tmux shows the error in the pane, and `remain-on-exit` keeps it visible. No special handling needed.

- **[Risk] Terminal size after split**: The split halves the horizontal width. If the terminal is narrow, both panes may be too small. -> **Mitigation**: This is the same trade-off as manual `Ctrl+b %`. Users can resize with tmux shortcuts. No mitigation needed from AoE.

- **[Risk] Sandboxed sessions with right pane**: For sandboxed sessions, the right pane tool should also run inside the container. -> **Mitigation**: For sandboxed sessions, the right pane command should be wrapped with the same `docker exec` invocation as the main command. This requires passing sandbox info to the split-window logic.
