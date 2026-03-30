## Context

When AoE creates a tmux session for an agent, the session's initial working directory is set to the project path via `new-session -c <path>`. However, tmux's default `%` and `"` split-window bindings do not pass `-c`, so new panes inherit the tmux server's default directory (typically `$HOME`), not the session's project path.

AoE already stores session-scoped custom options (e.g., `@aoe_agent_pane`, `@aoe_profile`) and overrides tmux keybindings with if-shell guards to distinguish AoE-managed sessions from non-AoE sessions. This change extends both patterns.

## Goals / Non-Goals

**Goals:**
- Manually split panes in AoE-managed sessions open in the session's project directory.
- Existing sessions gain the behavior on next TUI launch without recreation.
- Non-AoE sessions are unaffected; default `%` and `"` behavior is preserved.
- Default bindings are restored when AoE exits.

**Non-Goals:**
- Propagating the agent's *current* directory (which may differ from the project root). The project path is the stable, useful default.
- Supporting per-pane working directory overrides. Users can `cd` after splitting.
- Changing the working directory of the agent pane itself.

## Decisions

### Decision 1: Store project path as `@aoe_project_path` session option

**Choice**: Set `@aoe_project_path` via `set-option -t <session>` during session creation, alongside the existing `@aoe_agent_pane` store.

**Rationale**: Session-scoped user options (`@` prefix) are the established pattern in the codebase for attaching metadata to AoE sessions. The project path is already available as the `working_dir` parameter in `create_with_size()`. No format expansion (`-F`) is needed since the value is a plain path string.

**Alternative considered**: Querying the pane's `pane_current_path` at split time. Rejected because the agent process's cwd may not be the project root, and the additional format evaluation adds complexity.

### Decision 2: Use `append_store_pane_id_args` pattern for atomic creation

**Choice**: Add a new helper `append_store_project_path_args(args, target, working_dir)` that appends `; set-option -t <target> @aoe_project_path <path>` to the `new-session` argument vector, called from `create_with_size()` right after `append_store_pane_id_args()`.

**Rationale**: Keeps the session option store atomic with session creation (same tmux command chain). Follows the exact pattern of `append_store_pane_id_args`.

### Decision 3: Override `%` and `"` in the prefix table with if-shell guard

**Choice**: In `setup_session_cycle_bindings()`, add:
```
bind-key % if-shell -F "#{m:aoe_*,#{session_name}}" "split-window -h -c '#{@aoe_project_path}'" "split-window -h"
bind-key " if-shell -F "#{m:aoe_*,#{session_name}}" "split-window -v -c '#{@aoe_project_path}'" "split-window -v"
```

**Rationale**: The `if-shell -F` with `#{m:aoe_*,...}` pattern is already proven for `C-.` and `C-,` bindings. The `#{@aoe_project_path}` format is expanded by tmux at execution time from the session option. The fallback branch preserves default tmux behavior for non-AoE sessions.

**Alternative considered**: Using `split-window -c "#{pane_current_path}"` as fallback. Rejected to keep the fallback identical to tmux defaults and avoid unexpected behavior changes in non-AoE sessions.

### Decision 4: Restore defaults in cleanup (not unbind)

**Choice**: In `cleanup_session_cycle_bindings()`, restore the tmux default bindings:
```
bind-key % split-window -h
bind-key '"' split-window -v
```

**Rationale**: Unlike custom keys (C-., C-,) which should be unbound, `%` and `"` are standard tmux bindings that users expect to keep working after AoE exits. Restoring defaults instead of unbinding preserves the user's tmux experience.

### Decision 5: Backfill via `collect_tag_sessions_with_profile()`

**Choice**: In the existing `collect_tag_sessions_with_profile()` loop that iterates all instances, also emit `set-option -t <session> @aoe_project_path <path>` for each session.

**Rationale**: This function already runs on every `setup_session_cycle_bindings()` call (every TUI launch / attach cycle). Adding the project path store here means existing sessions created before this feature get the option set on next launch. The instance's `project_path` field is already available in the loop.

## Risks / Trade-offs

- **[Risk] Paths with special characters**: Project paths containing single quotes or spaces could break the tmux command string. **Mitigation**: Use `shell_escape()` (already available in utils.rs) when interpolating paths into tmux commands, and use tmux's `#{}` format expansion for the bind-key commands (which handles quoting internally).
- **[Risk] Stale project path after move**: If a user moves their project directory, `@aoe_project_path` will point to the old location until the session is recreated. **Mitigation**: Acceptable because the project path is already used throughout AoE and stale paths are a pre-existing condition. The backfill refreshes the value on every TUI launch.
- **[Trade-off] Prefix-table binding override**: Overriding `%` and `"` in the prefix table affects all sessions while AoE is running, not just AoE-managed ones. The if-shell guard ensures non-AoE sessions get default behavior, but the binding definition itself is global. This is consistent with how C-. and C-, are handled.
