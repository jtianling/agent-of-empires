## Why

AoE already binds `Ctrl+b h/j/k/l` for directional pane navigation, but there is no quick toggle binding to jump to the last-active pane (the equivalent of tmux's `Ctrl+b ;`). In a typical left/right split workflow, users want a single keystroke to bounce between two panes without thinking about direction. `Ctrl+Tab` is a natural, muscle-memory-friendly choice (similar to window/tab switching in most apps).

## What Changes

- Bind `Ctrl+Tab` (in the tmux prefix table) to `last-pane` within AoE-managed sessions.
- Follow the existing keybinding lifecycle: register in `setup_session_cycle_bindings()`, override in `apply_managed_session_bindings()` if needed, clean up on detach.
- The binding is purely a tmux `last-pane` command with no AoE-specific logic (no session switching, no profile resolution), so no CLI changes or new subcommands are needed.

## Capabilities

### New Capabilities
- `pane-last-toggle`: Ctrl+Tab keybinding that switches to the last-active pane within the current tmux window (equivalent to `Ctrl+b ;`).

### Modified Capabilities

## Impact

- `src/tmux/utils.rs`: Add binding in `setup_session_cycle_bindings()` and cleanup in `cleanup_session_cycle_bindings()`.
- No CLI, TUI, or config changes required -- this is a static tmux binding with no user-configurable parameters.
