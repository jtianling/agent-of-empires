## Why

Managing many AoE sessions is cumbersome. The existing `Ctrl+b n`/`p` group-based cycling works for adjacent sessions but is slow for jumping to a specific session. Users need a way to directly jump to any session by its number, both from the TUI list view and from within an attached tmux session.

## What Changes

- Display numeric indices (1-99) next to each session in the TUI list view. Only sessions get numbers; group headers do not. Numbering follows the global visible order (collapsed groups' sessions are skipped).
- In the TUI list view, pressing a digit key enters a "jump pending" state. A second digit auto-confirms (two-digit jump). Space confirms a single-digit jump. Any other key cancels.
- In attached tmux sessions, `Ctrl+b 1` through `Ctrl+b 9` enter tmux key tables (`aoe-1` through `aoe-9`). Within the key table, a second digit auto-confirms the two-digit target. Space confirms a single-digit target. Any unbound key cancels.
- Jump targets are global (cross-group), unlike `n`/`p` which cycle within the current group.

## Capabilities

### New Capabilities
- `number-jump`: Numeric session jumping from TUI list and tmux keybindings (1-99 range, Space to confirm single digit, second digit auto-confirms)

### Modified Capabilities
- `tui`: TUI list view gains numeric index display and digit-key input handling
- `session-management`: Session switching gains index-based jump (new `--index` CLI parameter)

## Impact

- `src/tui/home/render.rs`: Add numeric index rendering in session list
- `src/tui/home/input.rs`: Add digit key handling with pending-jump state
- `src/tui/home/mod.rs`: Add `PendingJump` state to `HomeView`
- `src/tmux/utils.rs`: Add 1-9 key table bindings in `setup_session_cycle_bindings()` and `apply_managed_session_bindings()`, cleanup in `cleanup_session_cycle_bindings()` and `cleanup_nested_detach_binding()`
- `src/tmux/utils.rs`: Add `switch_aoe_session_by_index()` function
- `src/cli/tmux.rs` (or equivalent): Add `--index N` parameter to `switch-session` subcommand
- `src/tmux/status_bar.rs`: Update status bar hints to show `1-9 jump`
