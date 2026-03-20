## Context

When an agent process exits, the pane shows "Pane is dead" (due to `remain-on-exit on`). The current attach-time recovery path in `app.rs:494-499` calls `tmux_session.kill()` which runs `kill-session`, destroying the entire tmux session including user-created panes and layout. AoE already tracks the agent pane via `@aoe_agent_pane`, and all status checks (`is_pane_dead`, `is_pane_running_shell`) already target this specific pane. The detection is precise, but the recovery is coarse.

Current keybindings: `r` = rename session. `R` (Shift+R) is unbound.

## Goals / Non-Goals

**Goals:**
- Add `R` keybinding to restart only the AoE-managed agent pane, preserving session layout
- Change attach-time recovery to prefer `respawn-pane` over `kill-session` when the session has user-created panes
- Keep process tree cleanup scoped to the agent pane only during respawn

**Non-Goals:**
- Changing the full session restart behavior (existing kill+recreate path stays for single-pane sessions or explicit destroy)
- Adding shell fallback after agent exit (the `exec` in `wrap_command_ignore_suspend` stays)
- Auto-restart without user action

## Decisions

### 1. Use `tmux respawn-pane -k` for agent-only restart

`respawn-pane -k -t <pane_id> <command>` kills the dead pane's process and starts a new one in the same pane. This preserves the session, all other panes, and the layout.

**Why not `respawn-pane` without `-k`?** Without `-k`, it only works on dead panes. With `-k`, it works on both dead and alive panes, making it usable for force-restart too.

**Why not remove `exec` from `wrap_command_ignore_suspend`?** That would cause AoE to misdetect the pane state (`is_pane_running_shell()` treats shell-in-pane as an error). The `respawn-pane` approach avoids changing the detection model.

### 2. Extract command construction from `start_with_size_opts()`

The agent launch command (binary, extra_args, yolo flags, env vars, custom instruction) is currently built inline in `start_with_size_opts()`. Extract it into a method like `build_agent_command() -> Option<String>` so both `start_with_size_opts()` and the new `respawn_agent_pane()` can reuse it.

On-launch hooks should also run during respawn, same as during initial start.

### 3. Attach-time recovery: respawn when multi-pane, kill when single-pane

In `app.rs` attach logic, when pane is dead:
- If session has only 1 pane: use existing `kill-session` + recreate (same as today)
- If session has >1 pane: use `respawn-pane` to preserve layout

This is determined by counting panes via `tmux list-panes -t <session> | wc -l`.

**Why keep kill+recreate for single-pane?** No layout to preserve, and recreating from scratch ensures a clean state (fresh tmux options, bindings, etc.).

### 4. Scoped process cleanup during respawn

Current `kill()` iterates `all_pane_pids()` and kills every process tree. For respawn, only kill the agent pane's process tree (using the PID from `@aoe_agent_pane`).

### 5. TUI `R` keybinding triggers respawn directly

`R` in home screen calls `respawn_agent_pane()` on the selected session. If the session doesn't exist yet, falls through to normal start. If the agent pane is alive, `respawn-pane -k` force-restarts it.

Status transitions to `Starting` after respawn, same as initial start.

## Risks / Trade-offs

- **[Respawn may not re-apply all tmux session options]** -> Mitigate by calling `apply_tmux_options()` after respawn, same as after create.
- **[Right pane (AoE-managed split) is not restarted]** -> Acceptable. `R` only targets the agent pane. Right pane has its own lifecycle.
- **[`respawn-pane` reuses the same pane size]** -> This is actually desirable; the pane keeps its current dimensions within the layout.
- **[On-launch hooks re-run on respawn]** -> Intentional. Hooks may set up env or config that the agent needs.
