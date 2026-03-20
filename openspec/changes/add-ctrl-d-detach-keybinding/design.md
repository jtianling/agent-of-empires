## Context

AoE manages tmux keybindings dynamically at runtime. When the user is inside an AoE-managed session (`aoe_*`), prefix bindings (`Ctrl+b d/n/p/h/j/k/l`) are set via `apply_managed_session_bindings()`. A `client-session-changed` hook re-evaluates bindings when switching sessions, restoring defaults for non-managed sessions.

Currently, returning to the AoE TUI requires the two-key prefix sequence `Ctrl+b d`. The user wants `Ctrl+d` (single chord, no prefix) as a faster alternative.

## Goals / Non-Goals

**Goals:**
- Add `Ctrl+d` in the tmux root key table as an alias for the existing detach-to-AoE logic
- Only active inside `aoe_*` sessions; transparent in non-managed sessions
- Same lifecycle as existing prefix bindings (set on attach, refreshed by hook, cleaned on exit)

**Non-Goals:**
- Replacing `Ctrl+b d` (it stays as-is)
- Making keybindings user-configurable (separate feature)
- Changing non-nested mode behavior beyond adding the root binding

## Decisions

### Decision 1: Use tmux root key table with `if-shell` guard

Bind `C-d` in the root table (`bind-key -T root C-d ...`). Use `if-shell` to check if the current session is AoE-managed:
- If managed: run the same `detach_run_shell_cmd()` shell script
- If not managed: `send-keys C-d` to pass through to the application (EOF, vim scroll, etc.)

**Why over alternatives:**
- **Alternative A: Bind/unbind on session change** - Simpler but creates a race window where `Ctrl+d` is swallowed during the session-changed hook. The `if-shell` approach is stateless and always correct.
- **Alternative B: Only bind in managed sessions** - tmux root bindings are global, not session-scoped. We cannot scope them to specific sessions without `if-shell`.

### Decision 2: Mirror the existing binding lifecycle

Add the root `C-d` binding in `apply_managed_session_bindings()` and remove it in `cleanup_nested_detach_binding()` and `cleanup_session_cycle_bindings()`. The `client-session-changed` hook already calls `refresh-bindings` which calls these functions, so no new hook logic is needed.

For cleanup: unbind from root table (`unbind-key -T root C-d`).

### Decision 3: Update status bar hint

Change the status bar hint from `Ctrl+b d detach` to `Ctrl+d detach` since the shorter binding is the primary one users should learn. `Ctrl+b d` still works but does not need prominent display.

## Risks / Trade-offs

- **`Ctrl+d` conflicts with shell EOF**: Mitigated by `if-shell` guard that sends `C-d` through to the application when not in an `aoe_*` session. Inside managed sessions, `Ctrl+d` for EOF is lost, but this is acceptable since the agent pane is the primary interaction and detach is more valuable there.
- **Root table binding persists if AoE crashes**: `cleanup_nested_detach_binding` may not run. Mitigated by the `if-shell` guard, the binding is harmless (sends passthrough `C-d`) after the managed session is gone.
