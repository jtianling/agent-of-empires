## Why

Grouped AoE sessions currently inherit `Ctrl+b j/k` cycling across the full profile order, so a
client attached from one group can jump into unrelated sessions in other groups. Once that happens,
`Ctrl+b d` no longer behaves like a reliable "return to AoE" action from the user's perspective,
because the user can be left cycling through sessions that were never part of the original group
context.

## What Changes

- Restrict `Ctrl+b j/k` cycling inside AoE-managed tmux sessions to the current session scope:
  sessions in the same group as the current session, or only other ungrouped sessions when the
  current session has no group.
- Preserve the original attach-origin AoE session as the `Ctrl+b d` return target even after the
  user cycles within the allowed scope.
- Add coverage for grouped cycling, ungrouped cycling, and returning to the AoE TUI after cycling
  inside nested tmux.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `nested-tmux-detach`: session cycling via `Ctrl+b j/k` becomes group-scoped instead of profile-
  wide, and `Ctrl+b d` must always return to the AoE session that initiated the attach flow after
  in-scope cycling.

## Impact

- `src/tmux/utils.rs`: scope-aware session ordering and detach target preservation for tmux key
  bindings
- `src/cli/tmux.rs`: hidden `switch-session` command may need additional scope context
- `src/tmux/session.rs` and `src/tmux/terminal_session.rs`: attach paths that seed nested tmux
  context
- `tests/` and `tests/e2e/`: grouped session cycling and nested detach regression coverage
